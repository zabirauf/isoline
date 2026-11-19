//! Terminals: a shell on a pseudo-terminal per tab (ConPTY on Windows).
//!
//! On macOS and Windows the shell's output goes to a native libghostty-vt terminal on its own
//! thread ([`engine`]); the embedder gets JSON frames of the rows that changed, at most 60 a
//! second and never more than it acknowledged, so output floods never reach the UI thread.
//! Input comes back as [`engine::Input`] and is encoded with Ghostty's encoders.
//!
//! On Linux (until the build has Zig for libghostty-vt) the embedder gets the raw output
//! bytes and draws them with ghostty-web, the same parser as WebAssembly.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use anyhow::{Context, Result, anyhow};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;

#[cfg(not(target_os = "linux"))]
pub mod engine;
#[cfg(not(target_os = "linux"))]
mod keys;

/// Whether this build parses terminal output natively (macOS, Windows).
pub const NATIVE: bool = cfg!(not(target_os = "linux"));

/// What a terminal sends its embedder.
pub enum Out {
    /// A frame, as JSON (native engine).
    Frame(String),
    /// Raw output (Linux).
    Bytes(Vec<u8>),
}

enum Sink {
    #[cfg(not(target_os = "linux"))]
    Engine(mpsc::Sender<engine::Msg>),
    #[cfg(target_os = "linux")]
    Pty(mpsc::Sender<Vec<u8>>),
}

struct Session {
    sink: Sink,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermInfo {
    pub id: u32,
    /// The shell's program name, such as "zsh" or "pwsh".
    pub shell: String,
    pub cwd: String,
}

/// How to start a terminal. Cell sizes are in device pixels (for mouse and selection math
/// and size reports); `theme` is an [`engine::Theme`] as JSON.
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u32,
    pub cell_h: u32,
    pub theme: Option<serde_json::Value>,
}

/// Every open terminal of the process.
#[derive(Default)]
pub struct Terminals {
    next: AtomicU32,
    sessions: Arc<Mutex<HashMap<u32, Session>>>,
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(windows)]
fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

/// The user's shell: `$SHELL` as a login shell (a GUI app's PATH lacks the profile's),
/// else zsh or bash. On Windows, PowerShell 7, else Windows PowerShell, else `%COMSPEC%`.
fn shell_command() -> CommandBuilder {
    #[cfg(windows)]
    {
        let exe = on_path("pwsh.exe")
            .or_else(|| on_path("powershell.exe"))
            .map(|p| p.into_os_string())
            .or_else(|| std::env::var_os("COMSPEC"))
            .unwrap_or_else(|| "cmd.exe".into());
        let mut c = CommandBuilder::new(&exe);
        if Path::new(&exe)
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("pwsh") || s.eq_ignore_ascii_case("powershell"))
        {
            c.arg("-NoLogo");
        }
        c
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| Path::new(s).is_file())
            .unwrap_or_else(|| {
                if Path::new("/bin/zsh").is_file() {
                    "/bin/zsh".into()
                } else {
                    "/bin/bash".into()
                }
            });
        let mut c = CommandBuilder::new(&shell);
        c.arg("-l");
        c
    }
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(2),
        cols: cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn spawn(name: String, f: impl FnOnce() + Send + 'static) -> Result<()> {
    std::thread::Builder::new()
        .name(name)
        .spawn(f)
        .map(|_| ())
        .context("could not start a terminal thread")
}

impl Terminals {
    /// Starts a shell in `cwd` (the home folder when there is none). `on_out` gets frames or
    /// bytes and returns false when the embedder is gone; `on_exit` gets the exit code once
    /// the shell ends, by itself or by [`Terminals::close`].
    pub fn open(
        &self,
        o: OpenOptions,
        on_out: impl FnMut(Out) -> bool + Send + 'static,
        on_exit: impl FnOnce(u32, Option<u32>) + Send + 'static,
    ) -> Result<TermInfo> {
        let dir = o.cwd.filter(|p| p.is_dir()).unwrap_or_else(home);
        let pty = native_pty_system()
            .openpty(pty_size(o.cols, o.rows))
            .map_err(|e| anyhow!("{e:#}"))?;
        let mut cmd = shell_command();
        cmd.cwd(&dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Isoline");
        if std::env::var_os("LANG").is_none() && !cfg!(windows) {
            cmd.env("LANG", "en_US.UTF-8");
        }
        let shell = Path::new(cmd.get_argv()[0].as_os_str())
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shell".into());
        let mut child = pty.slave.spawn_command(cmd).map_err(|e| anyhow!("{e:#}"))?;
        let killer = child.clone_killer();
        // The shell owns the slave side now; holding it would keep the PTY open after exit.
        drop(pty.slave);
        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|e| anyhow!("{e:#}"))?;
        let mut writer = pty.master.take_writer().map_err(|e| anyhow!("{e:#}"))?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;

        // Writer thread: input never waits on a shell that is not reading.
        let (pty_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
        spawn(format!("term-{id}-write"), move || {
            for b in pty_rx {
                if writer.write_all(&b).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })?;

        #[cfg(not(target_os = "linux"))]
        let sink = {
            let mut on_out = on_out;
            let theme: Option<engine::Theme> = o.theme.and_then(|v| serde_json::from_value(v).ok());
            let (tx, rx) = mpsc::channel::<engine::Msg>();
            let (cols, rows, cw, ch) = (o.cols, o.rows, o.cell_w.max(1), o.cell_h.max(1));
            spawn(format!("term-{id}-vt"), move || {
                engine::run(cols, rows, cw, ch, theme, rx, pty_tx, |f| {
                    serde_json::to_string(f)
                        .ok()
                        .is_some_and(|j| on_out(Out::Frame(j)))
                });
            })?;
            let out = tx.clone();
            spawn(format!("term-{id}-read"), move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if out.send(engine::Msg::Output(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                    }
                }
            })?;
            Sink::Engine(tx)
        };
        #[cfg(target_os = "linux")]
        let sink = {
            let mut on_out = on_out;
            let _ = (o.cell_w, o.cell_h, o.theme);
            spawn(format!("term-{id}-read"), move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if !on_out(Out::Bytes(buf[..n].to_vec())) {
                                break;
                            }
                        }
                    }
                }
            })?;
            Sink::Pty(pty_tx)
        };

        self.sessions.lock().insert(
            id,
            Session {
                sink,
                master: pty.master,
                killer,
            },
        );
        // Waits for the shell to exit, then closes the PTY: ConPTY does not end the output
        // stream by itself when the shell exits.
        let sessions = Arc::clone(&self.sessions);
        spawn(format!("term-{id}-wait"), move || {
            let code = child.wait().ok().map(|st| st.exit_code());
            if let Some(s) = sessions.lock().remove(&id) {
                stop(&s);
            }
            on_exit(id, code);
        })?;
        Ok(TermInfo {
            id,
            shell,
            cwd: dir.to_string_lossy().into_owned(),
        })
    }

    /// Input for a native terminal (an [`engine::Input`] as JSON).
    pub fn input(&self, id: u32, input: serde_json::Value) -> Result<()> {
        let g = self.sessions.lock();
        let s = g.get(&id).context("The terminal has closed.")?;
        #[cfg(not(target_os = "linux"))]
        {
            let Sink::Engine(tx) = &s.sink;
            let i: engine::Input = serde_json::from_value(input)?;
            if let engine::Input::Resize { cols, rows, .. } = &i {
                s.master
                    .resize(pty_size(*cols, *rows))
                    .map_err(|e| anyhow!("{e:#}"))?;
            }
            tx.send(engine::Msg::Input(i))
                .map_err(|_| anyhow!("The terminal has closed."))
        }
        #[cfg(target_os = "linux")]
        {
            let _ = (s, input);
            Err(anyhow!("This terminal takes raw input."))
        }
    }

    /// The selection as text (native terminals).
    pub fn copy(&self, id: u32) -> Result<Option<String>> {
        #[cfg(not(target_os = "linux"))]
        {
            let rx = {
                let g = self.sessions.lock();
                let s = g.get(&id).context("The terminal has closed.")?;
                let Sink::Engine(tx) = &s.sink;
                let (reply, rx) = mpsc::channel();
                tx.send(engine::Msg::Copy(reply))
                    .map_err(|_| anyhow!("The terminal has closed."))?;
                rx
            };
            Ok(rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .ok()
                .flatten())
        }
        #[cfg(target_os = "linux")]
        {
            let _ = id;
            Ok(None)
        }
    }

    /// Raw input: typed text for the ghostty-web path; text input for a native terminal.
    pub fn write(&self, id: u32, data: &str) -> Result<()> {
        let g = self.sessions.lock();
        let s = g.get(&id).context("The terminal has closed.")?;
        match &s.sink {
            #[cfg(target_os = "linux")]
            Sink::Pty(tx) => tx.send(data.as_bytes().to_vec()).ok(),
            #[cfg(not(target_os = "linux"))]
            Sink::Engine(tx) => tx
                .send(engine::Msg::Input(engine::Input::Text {
                    text: data.to_string(),
                }))
                .ok(),
        }
        .context("The terminal has closed.")
    }

    /// Resizes the PTY (the ghostty-web path; native terminals resize through `input`).
    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<()> {
        let g = self.sessions.lock();
        let s = g.get(&id).context("The terminal has closed.")?;
        s.master
            .resize(pty_size(cols, rows))
            .map_err(|e| anyhow!("{e:#}"))
    }

    /// Stops a shell. Its wait thread then calls `on_exit`.
    pub fn close(&self, id: u32) {
        let s = self.sessions.lock().remove(&id);
        if let Some(mut s) = s {
            let _ = s.killer.kill();
            stop(&s);
        }
    }
}

fn stop(s: &Session) {
    #[cfg(not(target_os = "linux"))]
    {
        let Sink::Engine(tx) = &s.sink;
        let _ = tx.send(engine::Msg::Stop);
    }
    #[cfg(target_os = "linux")]
    let _ = s;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The user's shell starts on a PTY in the given folder and runs what is typed.
    #[test]
    fn shell_runs_in_the_folder() {
        let dir = tempfile_dir();
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = shell_command();
        cmd.cwd(&dir);
        cmd.env("TERM", "xterm-256color");
        let mut child = pty.slave.spawn_command(cmd).unwrap();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().unwrap();
        let mut w = pty.master.take_writer().unwrap();
        let out = std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = reader.read_to_end(&mut all);
            String::from_utf8_lossy(&all).into_owned()
        });
        w.write_all(b"echo ISO-$((40+2)); pwd; exit 3\n").unwrap();
        w.flush().unwrap();
        let code = child.wait().unwrap().exit_code();
        drop(w);
        drop(pty.master);
        let text = out.join().unwrap();
        assert_eq!(code, 3);
        assert!(text.contains("ISO-42"), "{text}");
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(text.contains(&name), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("iso-term-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
