//! Terminals: a shell on a pseudo-terminal per tab (ConPTY on Windows), started in the repo
//! folder. Output streams to the webview over a Tauri channel as raw bytes; the UI draws it
//! with ghostty-web (Ghostty's VT parser compiled to WebAssembly). Input and resizes come
//! back as commands. A shell that exits sends `term_exit`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, CmdResult, err};

struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Default)]
pub struct TermState {
    next: AtomicU32,
    sessions: Arc<Mutex<HashMap<u32, Session>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermInfo {
    pub id: u32,
    /// The shell's program name, such as "zsh" or "pwsh".
    pub shell: String,
    pub cwd: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TermExit {
    id: u32,
    code: Option<u32>,
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

/// Starts a shell in `cwd` (the repo folder; the home folder when there is none).
#[tauri::command]
pub async fn term_open(
    app: AppHandle,
    state: State<'_, AppState>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<TermInfo> {
    let dir = cwd
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(home);
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: rows.max(2),
            cols: cols.max(10),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(err)?;
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
    let mut child = pty.slave.spawn_command(cmd).map_err(err)?;
    let killer = child.clone_killer();
    // The shell owns the slave side now; holding it here would keep the PTY open after exit.
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().map_err(err)?;
    let writer = pty.master.take_writer().map_err(err)?;
    let id = state.term.next.fetch_add(1, Ordering::Relaxed) + 1;
    state.term.sessions.lock().insert(
        id,
        Session {
            writer,
            master: pty.master,
            killer,
        },
    );
    std::thread::Builder::new()
        .name(format!("term-{id}-read"))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if on_data
                            .send(InvokeResponseBody::Raw(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .map_err(err)?;
    // Waits for the shell to exit, then closes the PTY: ConPTY does not end the output
    // stream by itself when the shell exits.
    let sessions = Arc::clone(&state.term.sessions);
    std::thread::Builder::new()
        .name(format!("term-{id}-wait"))
        .spawn(move || {
            let code = child.wait().ok().map(|st| st.exit_code());
            drop(sessions.lock().remove(&id));
            let _ = app.emit("term_exit", TermExit { id, code });
        })
        .map_err(err)?;
    Ok(TermInfo {
        id,
        shell,
        cwd: dir.to_string_lossy().into_owned(),
    })
}

/// Sends typed or pasted text to the shell.
#[tauri::command]
pub async fn term_write(state: State<'_, AppState>, id: u32, data: String) -> CmdResult<()> {
    let mut g = state.term.sessions.lock();
    let s = g.get_mut(&id).ok_or("The terminal has closed.")?;
    s.writer.write_all(data.as_bytes()).map_err(err)?;
    s.writer.flush().map_err(err)
}

#[tauri::command]
pub async fn term_resize(
    state: State<'_, AppState>,
    id: u32,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    let g = state.term.sessions.lock();
    let s = g.get(&id).ok_or("The terminal has closed.")?;
    s.master
        .resize(PtySize {
            rows: rows.max(2),
            cols: cols.max(10),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(err)
}

/// Stops the shell of a closed tab. Its wait thread then sends `term_exit`.
#[tauri::command]
pub async fn term_close(state: State<'_, AppState>, id: u32) -> CmdResult<()> {
    let s = state.term.sessions.lock().remove(&id);
    if let Some(mut s) = s {
        let _ = s.killer.kill();
    }
    Ok(())
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
