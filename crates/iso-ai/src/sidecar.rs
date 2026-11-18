//! The Copilot sidecar client: newline-delimited JSON-RPC 2.0 over the sidecar's stdio
//! (CONTRACTS.md section 6).
//!
//! The process starts lazily, on the first call. Requests carry ids, so several threads can
//! call at once. When the process exits or a call times out, pending calls fail and the
//! next call starts a fresh process; more than `max_restarts` restarts within
//! `restart_window` fail fast instead.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AiError, Completion, CompletionRequest, ModelInfo, Provider};

/// JSON-RPC error code for "not signed in".
pub const CODE_NOT_SIGNED_IN: i64 = -32001;
/// JSON-RPC error code for "sign-in needs a GitHub OAuth app client id".
pub const CODE_NO_CLIENT_ID: i64 = -32002;
/// JSON-RPC error code for "this sidecar was built without the Copilot SDK".
pub const CODE_LIVE_UNAVAILABLE: i64 = -32003;

/// How to run the sidecar.
#[derive(Clone, Debug)]
pub struct SidecarConfig {
    /// Path to `copilot-sidecar-<target triple>`.
    pub binary: PathBuf,
    /// Pass `--mock`: no network, deterministic answers.
    pub mock: bool,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Timeout for status, sign-in and model listing.
    pub call_timeout: Duration,
    /// Timeout for one completion.
    pub complete_timeout: Duration,
    pub max_restarts: u32,
    pub restart_window: Duration,
}

impl SidecarConfig {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        SidecarConfig {
            binary: binary.into(),
            mock: false,
            args: Vec::new(),
            env: Vec::new(),
            call_timeout: Duration::from_secs(60),
            complete_timeout: Duration::from_secs(600),
            max_restarts: 5,
            restart_window: Duration::from_secs(60),
        }
    }

    pub fn mock(mut self, mock: bool) -> Self {
        self.mock = mock;
        self
    }
}

/// Result of `status`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarStatus {
    pub signed_in: bool,
    #[serde(default)]
    pub user: Option<String>,
    pub mock: bool,
    pub version: String,
    /// Whether the Copilot SDK is built into this sidecar.
    #[serde(default)]
    pub live: bool,
}

/// Result of `signIn`: the device-flow code to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollResult {
    signed_in: bool,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

type Reply = Result<Value, RpcError>;

struct Process {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Reply>>>>,
    alive: Arc<AtomicBool>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl Process {
    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let mut child = self.child.lock();
        let _ = child.kill();
        let _ = child.wait();
    }

    fn tail(&self) -> String {
        let t = self.stderr_tail.lock();
        t.iter().cloned().collect::<Vec<_>>().join(" | ")
    }
}

/// A [`Provider`] backed by the Go Copilot sidecar.
pub struct SidecarProvider {
    cfg: SidecarConfig,
    proc: Mutex<Option<Arc<Process>>>,
    next_id: AtomicU64,
    starts: Mutex<VecDeque<Instant>>,
    spawned: AtomicU64,
}

impl SidecarProvider {
    /// Does not start the process; the first call does.
    pub fn new(cfg: SidecarConfig) -> Self {
        SidecarProvider {
            cfg,
            proc: Mutex::new(None),
            next_id: AtomicU64::new(1),
            starts: Mutex::new(VecDeque::new()),
            spawned: AtomicU64::new(0),
        }
    }

    pub fn config(&self) -> &SidecarConfig {
        &self.cfg
    }

    /// Whether a sidecar process is running now.
    pub fn is_running(&self) -> bool {
        self.proc
            .lock()
            .as_ref()
            .is_some_and(|p| p.alive.load(Ordering::SeqCst))
    }

    /// How many processes have been started so far.
    pub fn spawn_count(&self) -> u64 {
        self.spawned.load(Ordering::SeqCst)
    }

    fn spawn(&self) -> Result<Arc<Process>, AiError> {
        {
            let mut starts = self.starts.lock();
            let now = Instant::now();
            while starts
                .front()
                .is_some_and(|t| now.duration_since(*t) > self.cfg.restart_window)
            {
                starts.pop_front();
            }
            if starts.len() as u32 > self.cfg.max_restarts {
                return Err(AiError::Provider(format!(
                    "the sidecar exited {} times within {} s; not starting it again",
                    starts.len(),
                    self.cfg.restart_window.as_secs()
                )));
            }
            starts.push_back(now);
        }
        let mut cmd = Command::new(&self.cfg.binary);
        if self.cfg.mock {
            cmd.arg("--mock");
        }
        cmd.args(&self.cfg.args)
            .envs(self.cfg.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| {
            AiError::Provider(format!(
                "could not start the sidecar {}: {e}",
                self.cfg.binary.display()
            ))
        })?;
        self.spawned.fetch_add(1, Ordering::SeqCst);
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Reply>>>> = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();

        {
            let tail = tail.clone();
            let echo = std::env::var("ISOLINE_SIDECAR_LOG").is_ok_and(|v| v == "1");
            std::thread::Builder::new()
                .name("copilot-sidecar-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        let Ok(line) = line else { break };
                        if echo {
                            eprintln!("{line}");
                        }
                        let mut t = tail.lock();
                        if t.len() >= 20 {
                            t.pop_front();
                        }
                        t.push_back(line);
                    }
                })
                .map_err(|e| AiError::Provider(e.to_string()))?;
        }
        {
            let pending = pending.clone();
            let alive = alive.clone();
            std::thread::Builder::new()
                .name("copilot-sidecar-stdout".into())
                .spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        let Ok(v) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        let Some(id) = v.get("id").and_then(Value::as_u64) else {
                            continue;
                        };
                        let reply = match v.get("error") {
                            Some(e) if !e.is_null() => Err(RpcError {
                                code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                                message: e
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown error")
                                    .to_string(),
                            }),
                            _ => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        if let Some(tx) = pending.lock().remove(&id) {
                            let _ = tx.send(reply);
                        }
                    }
                    alive.store(false, Ordering::SeqCst);
                    // Dropping the senders wakes every waiting call.
                    pending.lock().clear();
                })
                .map_err(|e| AiError::Provider(e.to_string()))?;
        }
        Ok(Arc::new(Process {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            alive,
            stderr_tail: tail,
        }))
    }

    fn process(&self) -> Result<Arc<Process>, AiError> {
        let mut slot = self.proc.lock();
        if let Some(p) = slot.as_ref()
            && p.alive.load(Ordering::SeqCst)
        {
            return Ok(p.clone());
        }
        if let Some(old) = slot.take() {
            old.kill();
        }
        let p = self.spawn()?;
        *slot = Some(p.clone());
        Ok(p)
    }

    /// Sends one request and waits for its reply.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, AiError> {
        let p = self.process()?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        p.pending.lock().insert(id, tx);
        let line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        let written = {
            let mut stdin = p.stdin.lock();
            stdin
                .write_all(line.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush())
        };
        if let Err(e) = written {
            p.pending.lock().remove(&id);
            p.kill();
            return Err(AiError::Provider(format!(
                "the sidecar stopped reading requests: {e}"
            )));
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) if e.code == CODE_NOT_SIGNED_IN => Err(AiError::NotSignedIn),
            Ok(Err(e)) => Err(AiError::Provider(format!(
                "{} (code {})",
                e.message, e.code
            ))),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                p.pending.lock().remove(&id);
                // A hung sidecar is replaced on the next call.
                p.kill();
                Err(AiError::Provider(format!(
                    "the sidecar did not answer {method} within {} s",
                    timeout.as_secs()
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Give the stderr reader a moment to collect the last lines.
                std::thread::sleep(Duration::from_millis(50));
                let tail = p.tail();
                p.kill();
                Err(AiError::Provider(if tail.is_empty() {
                    "the sidecar exited".to_string()
                } else {
                    format!("the sidecar exited: {tail}")
                }))
            }
        }
    }

    fn typed<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, AiError> {
        let v = self.call(method, params, self.cfg.call_timeout)?;
        serde_json::from_value(v)
            .map_err(|e| AiError::Provider(format!("unexpected {method} reply: {e}")))
    }

    pub fn status(&self) -> Result<SidecarStatus, AiError> {
        self.typed("status", json!({}))
    }

    /// Starts the device flow. Without a client id the sidecar uses
    /// `ISOLINE_GITHUB_CLIENT_ID`, and fails with a clear error when neither is set.
    pub fn sign_in(&self, client_id: Option<&str>) -> Result<DeviceCode, AiError> {
        let params = match client_id {
            Some(c) => json!({ "clientId": c }),
            None => json!({}),
        };
        self.typed("signIn", params)
    }

    /// Polls the device flow once. Returns whether the user is signed in, and as whom.
    pub fn sign_in_poll(&self) -> Result<(bool, Option<String>), AiError> {
        let r: PollResult = self.typed("signInPoll", json!({}))?;
        Ok((r.signed_in, r.user))
    }

    pub fn sign_out(&self) -> Result<(), AiError> {
        let _: Value = self.typed("signOut", json!({}))?;
        Ok(())
    }

    /// Asks the sidecar to exit and waits briefly for it.
    pub fn shutdown(&self) {
        let p = self.proc.lock().take();
        if let Some(p) = p {
            if p.alive.load(Ordering::SeqCst) {
                let id = self.next_id.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = mpsc::channel();
                p.pending.lock().insert(id, tx);
                let line = json!({"jsonrpc": "2.0", "id": id, "method": "shutdown", "params": {}});
                let ok = {
                    let mut stdin = p.stdin.lock();
                    writeln!(stdin, "{line}")
                        .and_then(|_| stdin.flush())
                        .is_ok()
                };
                if ok {
                    let _ = rx.recv_timeout(Duration::from_secs(3));
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    let done = p.child.lock().try_wait().ok().flatten().is_some();
                    if done || Instant::now() > deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            p.kill();
        }
    }
}

impl Drop for SidecarProvider {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Deserialize)]
struct ModelList {
    models: Vec<ModelInfo>,
}

impl Provider for SidecarProvider {
    fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        let r: ModelList = self.typed("listModels", json!({}))?;
        Ok(r.models)
    }

    /// The sidecar answers each request in its own goroutine and Copilot session.
    fn max_concurrency(&self) -> usize {
        6
    }

    fn calibration(&self) -> crate::Calibration {
        if self.cfg.mock {
            crate::Calibration::EXACT
        } else {
            crate::Calibration::COPILOT
        }
    }

    fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError> {
        let params = serde_json::to_value(req).map_err(|e| AiError::Provider(e.to_string()))?;
        let v = self.call("complete", params, self.cfg.complete_timeout)?;
        serde_json::from_value(v)
            .map_err(|e| AiError::Provider(format!("unexpected complete reply: {e}")))
    }
}
