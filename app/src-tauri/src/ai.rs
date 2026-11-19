//! AI commands. The sidecar starts only after the user turns AI on for a repo; cached
//! summaries are served from the shared cache at all times without starting it.
//!
//! Sidecar binary: `ISOLINE_SIDECAR_PATH`, else `copilot-sidecar` next to the app binary
//! (Tauri's externalBin), else the dev build in `sidecar/copilot/bin/`. Set
//! `ISOLINE_SIDECAR_MOCK=1` to run it in mock mode.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use iso_ai::{AiEngine, AiPrefs, AiSettings, SidecarConfig, SidecarProvider};
use iso_core::ipc::*;
use iso_engine::{AiLayer, Engine};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, CmdResult, blocking, err};

#[derive(Default)]
pub struct AiState {
    inner: Mutex<Option<(Arc<SidecarProvider>, Arc<AiEngine>)>>,
    device: Mutex<Option<DeviceCode>>,
    /// Why the last sign-in stopped (code expired, denied, GitHub unreachable), shown until
    /// the next sign-in starts.
    notice: Mutex<Option<String>>,
    /// Bumped by every sign-in and cancel; a poller whose sign-in is no longer current stops.
    sign_in_gen: AtomicU64,
    next_job: AtomicU64,
}

/// Kept for the app's lifetime: on Linux the copied text lives only as long as its owner.
static CLIPBOARD: std::sync::OnceLock<Mutex<Option<arboard::Clipboard>>> =
    std::sync::OnceLock::new();

fn copy_text(text: &str) -> bool {
    let cell = CLIPBOARD.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()));
    let mut g = cell.lock();
    g.as_mut().is_some_and(|c| c.set_text(text).is_ok())
}

fn sidecar_path() -> PathBuf {
    if let Ok(p) = std::env::var("ISOLINE_SIDECAR_PATH") {
        return PathBuf::from(p);
    }
    let exe_name = if cfg!(windows) {
        "copilot-sidecar.exe"
    } else {
        "copilot-sidecar"
    };
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        let p = dir.join(exe_name);
        if p.is_file() {
            return p;
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/copilot/bin");
    std::fs::read_dir(&dev)
        .ok()
        .and_then(|rd| {
            rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("copilot-sidecar"))
            })
        })
        .unwrap_or_else(|| dev.join(exe_name))
}

fn mock_mode() -> bool {
    std::env::var("ISOLINE_SIDECAR_MOCK").is_ok_and(|v| v == "1" || v == "true")
}

impl AiState {
    /// The shared AI engine. Creating it starts no process.
    fn get(&self) -> anyhow::Result<(Arc<SidecarProvider>, Arc<AiEngine>)> {
        let mut g = self.inner.lock();
        if let Some(x) = g.as_ref() {
            return Ok(x.clone());
        }
        let provider = Arc::new(SidecarProvider::new(
            SidecarConfig::new(sidecar_path()).mock(mock_mode()),
        ));
        let ai = Arc::new(AiEngine::open(
            &iso_engine::data_dir().join("ai_cache.sqlite"),
            provider.clone(),
        )?);
        // The models and prompts chosen in Settings, for every repo.
        ai.set_settings(AiPrefs::load(&prefs_file()).apply(ai.settings()));
        *g = Some((provider.clone(), ai.clone()));
        Ok((provider, ai))
    }
}

/// Serve cached summaries in this repo's file views.
pub(crate) fn attach(app: &AppHandle, eng: &Arc<Engine>) {
    use tauri::Manager;
    let st = app.state::<AppState>();
    if let Ok((_, ai)) = st.ai.get() {
        eng.set_ai_hook(Some(Arc::new(AiLayer { ai })));
    }
}

/// Models and prompts chosen in Settings.
fn prefs_file() -> PathBuf {
    iso_engine::data_dir().join("ai_settings.json")
}

/// The GitHub login of the last sign-in, kept beside the AI cache so the app knows it is
/// signed in without starting the sidecar (the token itself stays in the OS keychain).
fn account_file() -> PathBuf {
    iso_engine::data_dir().join("ai_account")
}

fn remembered_account() -> Option<String> {
    let s = std::fs::read_to_string(account_file()).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn remember_account(user: Option<&str>) {
    let path = account_file();
    match user {
        Some(u) if !u.is_empty() => {
            if remembered_account().as_deref() != Some(u) {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, u);
            }
        }
        _ => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub(crate) fn status(state: &AppState, message: Option<String>) -> AiStatus {
    let enabled = state.engine.read().as_ref().is_some_and(|e| e.ai_enabled());
    let device_code = state.ai.device.lock().clone();
    let mut s = AiStatus {
        sidecar: SidecarState::Off,
        signed_in: false,
        user: None,
        models: vec![],
        enabled_for_repo: enabled,
        mock: mock_mode(),
        message: message.or_else(|| state.ai.notice.lock().clone()),
        device_code,
    };
    // Ask the sidecar once AI is on for this repo, while a sign-in is pending (which comes
    // before turning AI on), or when it already runs. Otherwise don't start it.
    let running = state
        .ai
        .inner
        .lock()
        .as_ref()
        .is_some_and(|(p, _)| p.is_running());
    if !enabled && s.device_code.is_none() && !running {
        // Not started: report the sign-in from an earlier session, so turning AI on for
        // this repo is one click and needs no new sign-in.
        if let Some(user) = remembered_account() {
            s.signed_in = true;
            s.user = Some(user);
        }
        return s;
    }
    let Ok((provider, ai)) = state.ai.get() else {
        s.sidecar = SidecarState::Failed;
        s.message = Some("The AI cache could not be opened.".into());
        return s;
    };
    match provider.status() {
        Ok(st) => {
            s.sidecar = SidecarState::Ready;
            s.signed_in = st.signed_in;
            s.user = st.user;
            s.mock = st.mock;
            if !st.mock {
                remember_account(
                    st.signed_in
                        .then_some(s.user.as_deref().unwrap_or("GitHub")),
                );
            }
            if st.signed_in {
                *state.ai.device.lock() = None;
                s.device_code = None;
                if let Ok(models) = ai.refresh_models() {
                    ai.set_settings(AiSettings { ..ai.settings() }.fitted_to(&models));
                    s.models = models
                        .into_iter()
                        .map(|m| AiModel {
                            id: m.id,
                            name: m.name,
                            input_per_m: m.input_per_m,
                            output_per_m: m.output_per_m,
                        })
                        .collect();
                }
            }
        }
        Err(e) => {
            s.sidecar = SidecarState::Failed;
            s.message = Some(format!("The Copilot sidecar did not start: {e}"));
        }
    }
    s
}

#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    Ok(status(&state, None))
}

/// Polls GitHub until the device code is approved, denied or expired, then clears it, so the
/// next `ai_status` shows the outcome. Gives up after three failed polls in a row.
fn poll_sign_in(app: AppHandle, provider: Arc<SidecarProvider>, generation: u64) {
    use tauri::Manager;
    let current = |app: &AppHandle| {
        app.state::<AppState>()
            .ai
            .sign_in_gen
            .load(Ordering::SeqCst)
            == generation
    };
    let mut failures = 0;
    let mut outcome: Option<String> = Some("The sign-in code expired. Sign in again.".into());
    for _ in 0..180 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if !current(&app) {
            return;
        }
        match provider.sign_in_poll() {
            Ok((true, _)) => {
                outcome = None;
                break;
            }
            Ok((false, _)) => failures = 0,
            Err(e) => {
                let msg = e.to_string();
                failures += 1;
                if failures >= 3 || msg.contains("expired") || msg.contains("denied") {
                    outcome = Some(format!("Sign-in stopped: {msg}"));
                    break;
                }
            }
        }
    }
    if !current(&app) {
        return;
    }
    let state = app.state::<AppState>();
    *state.ai.device.lock() = None;
    *state.ai.notice.lock() = outcome;
    let _ = app.emit("ai_status", status(&state, None));
}

#[tauri::command]
pub async fn ai_enable(state: State<'_, AppState>, enabled: bool) -> CmdResult<AiStatus> {
    let eng = state.engine.read().clone();
    if let Some(e) = eng {
        e.set_ai_enabled(enabled).map_err(err)?;
    }
    if !enabled && let Some((p, _)) = state.ai.inner.lock().as_ref() {
        p.shutdown();
    }
    Ok(status(&state, None))
}

#[tauri::command]
pub async fn ai_sign_in(app: AppHandle, state: State<'_, AppState>) -> CmdResult<AiStatus> {
    let (provider, _) = state.ai.get().map_err(err)?;
    *state.ai.notice.lock() = None;
    // A token from an earlier session: already signed in, no new code needed.
    if provider.status().is_ok_and(|st| st.signed_in) {
        *state.ai.device.lock() = None;
        return Ok(status(&state, None));
    }
    let client = std::env::var("ISOLINE_GITHUB_CLIENT_ID").ok();
    match provider.sign_in(client.as_deref()) {
        Ok(code) => {
            // Put the code on the clipboard and open GitHub's page, so finishing is a paste.
            let copied = copy_text(&code.user_code);
            let opened = code.verification_uri.starts_with("https://github.com/") && {
                use tauri_plugin_opener::OpenerExt;
                app.opener()
                    .open_url(&code.verification_uri, None::<&str>)
                    .is_ok()
            };
            let hint = match (copied, opened) {
                (true, true) => {
                    "The code is on your clipboard and GitHub is open in your browser: paste it there."
                }
                (true, false) => "The code is on your clipboard: paste it at the address above.",
                (false, true) => "GitHub is open in your browser: enter the code there.",
                (false, false) => "Enter the code on GitHub to finish signing in.",
            };
            *state.ai.device.lock() = Some(DeviceCode {
                user_code: code.user_code,
                verification_uri: code.verification_uri,
            });
            let generation = state.ai.sign_in_gen.fetch_add(1, Ordering::SeqCst) + 1;
            let (app2, p) = (app.clone(), Arc::clone(&provider));
            std::thread::spawn(move || poll_sign_in(app2, p, generation));
            Ok(status(&state, Some(hint.into())))
        }
        Err(e) => Ok(status(&state, Some(format!("{e}")))),
    }
}

/// Stops waiting for a pending sign-in (the dialog was closed). A code approved on GitHub
/// afterwards is not picked up; signing in again starts a new one.
#[tauri::command]
pub async fn ai_sign_in_cancel(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    state.ai.sign_in_gen.fetch_add(1, Ordering::SeqCst);
    *state.ai.device.lock() = None;
    *state.ai.notice.lock() = None;
    Ok(status(&state, None))
}

#[tauri::command]
pub async fn ai_sign_out(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    if let Ok((p, _)) = state.ai.get() {
        let _ = p.sign_out();
    }
    remember_account(None);
    *state.ai.device.lock() = None;
    *state.ai.notice.lock() = None;
    Ok(status(&state, None))
}

#[tauri::command]
pub async fn ai_estimate(
    state: State<'_, AppState>,
    scope: AiScope,
    path: Option<String>,
) -> CmdResult<AiEstimate> {
    let eng = crate::engine(&state)?;
    let (_, ai) = state.ai.get().map_err(err)?;
    blocking(move || eng.ai_estimate(&ai, scope, path.as_deref())).await
}

/// The cached AI review of a comparison (the payload `compare` returned), else its cost.
#[tauri::command]
pub async fn ai_review(
    state: State<'_, AppState>,
    comparison: Comparison,
) -> CmdResult<ReviewState> {
    let eng = crate::engine(&state)?;
    let (_, ai) = state.ai.get().map_err(err)?;
    blocking(move || eng.ai_review_state(&ai, &comparison)).await
}

/// Writes the AI review of a comparison within the budget: one request, answered when done.
#[tauri::command]
pub async fn ai_review_run(
    state: State<'_, AppState>,
    comparison: Comparison,
    budget_usd: f64,
) -> CmdResult<ChangeReview> {
    let eng = crate::engine(&state)?;
    let (_, ai) = state.ai.get().map_err(err)?;
    blocking(move || eng.ai_review_run(&ai, &comparison, budget_usd)).await
}

#[tauri::command]
pub async fn ai_run(
    app: AppHandle,
    state: State<'_, AppState>,
    scope: AiScope,
    path: Option<String>,
    budget_usd: f64,
) -> CmdResult<AiRunHandle> {
    let eng = crate::engine(&state)?;
    let (_, ai) = state.ai.get().map_err(err)?;
    let estimate = {
        let (eng, ai, path) = (Arc::clone(&eng), Arc::clone(&ai), path.clone());
        blocking(move || eng.ai_estimate(&ai, scope, path.as_deref())).await?
    };
    if estimate.cost_usd > budget_usd {
        return Err(format!(
            "This run is estimated at ${:.2}, over the budget of ${budget_usd:.2}.",
            estimate.cost_usd
        ));
    }
    let job_id = state.ai.next_job.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        let h = app.clone();
        let res = eng.ai_run(&ai, scope, path.as_deref(), budget_usd, &mut |p| {
            let _ = h.emit("ai_progress", p.to_ipc(job_id));
        });
        // One event per job, after every pass: per-pass progress also says "finished".
        let done = match &res {
            Ok(rep) => {
                let l = &rep.leaf;
                let cost = l.cost_usd
                    + rep.cluster.as_ref().map_or(0.0, |c| c.run.cost_usd)
                    + rep.repo.as_ref().map_or(0.0, |r| r.run.cost_usd);
                let error = if l.stopped_by_budget {
                    Some("Stopped at the budget.".to_string())
                } else if l.stored == 0 && l.failed > 0 {
                    Some(format!(
                        "No summary could be used: {}",
                        l.errors
                            .first()
                            .map_or("no reason given", |e| e.message.as_str())
                    ))
                } else {
                    None
                };
                serde_json::json!({ "jobId": job_id, "stored": l.stored, "failed": l.failed, "costUsd": cost, "error": error })
            }
            Err(e) => {
                serde_json::json!({ "jobId": job_id, "stored": 0, "failed": 0, "costUsd": 0.0, "error": format!("{e:#}") })
            }
        };
        match res {
            Ok(rep) => {
                for p in &rep.leaf.updated_paths {
                    let _ = app.emit("summary_ready", serde_json::json!({ "path": p }));
                }
                if rep.cluster.is_some()
                    && let Ok(s) = eng.summary()
                {
                    let _ = app.emit("index_done", s);
                }
            }
            Err(e) => {
                let _ = app.emit(
                    "ai_progress",
                    AiProgress {
                        job_id,
                        done: 0,
                        total: 0,
                        tokens_in: 0,
                        tokens_out: 0,
                        cost_usd: 0.0,
                        finished: true,
                        error: Some(format!("{e:#}")),
                    },
                );
            }
        }
        let _ = app.emit("ai_job_done", done);
    });
    Ok(AiRunHandle { job_id, estimate })
}

/// What Settings shows: each task's model and prompt, and the models to choose from. Lists
/// the account's models when signed in (this starts the sidecar but sends no code).
#[tauri::command]
pub async fn ai_config(state: State<'_, AppState>) -> CmdResult<AiConfig> {
    let (provider, ai) = state.ai.get().map_err(err)?;
    let signed_in = remembered_account().is_some() || provider.is_running() || mock_mode();
    blocking(move || {
        if signed_in && !ai.known_models().1 && provider.status().is_ok_and(|s| s.signed_in) {
            let _ = ai.refresh_models();
        }
        let (models, listed) = ai.known_models();
        Ok(iso_ai::prefs::config(&ai.settings(), &models, listed))
    })
    .await
}

/// Saves the models and prompts for every repo; the next request uses them.
#[tauri::command]
pub async fn ai_set_config(
    state: State<'_, AppState>,
    update: AiConfigUpdate,
) -> CmdResult<AiConfig> {
    let (_, ai) = state.ai.get().map_err(err)?;
    let (s, prefs) = AiPrefs::update(ai.settings(), &update);
    prefs.save(&prefs_file()).map_err(err)?;
    ai.set_settings(s);
    let (models, listed) = ai.known_models();
    Ok(iso_ai::prefs::config(&ai.settings(), &models, listed))
}

/// The model requests of this session, newest first (developer panel).
#[tauri::command]
pub async fn ai_requests(state: State<'_, AppState>) -> CmdResult<Vec<AiRequestRow>> {
    let (_, ai) = state.ai.get().map_err(err)?;
    Ok(ai.requests())
}

#[tauri::command]
pub async fn ai_request(state: State<'_, AppState>, id: u32) -> CmdResult<Option<AiRequestDetail>> {
    let (_, ai) = state.ai.get().map_err(err)?;
    Ok(ai.request(id))
}

#[tauri::command]
pub async fn ai_requests_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let (_, ai) = state.ai.get().map_err(err)?;
    ai.clear_requests();
    Ok(())
}
