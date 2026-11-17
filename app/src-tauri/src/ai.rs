//! AI commands. The sidecar starts only after the user turns AI on for a repo; cached
//! summaries are served from the shared cache at all times without starting it.
//!
//! Sidecar binary: `ISOLINE_SIDECAR_PATH`, else `copilot-sidecar` next to the app binary
//! (Tauri's externalBin), else the dev build in `sidecar/copilot/bin/`. Set
//! `ISOLINE_SIDECAR_MOCK=1` to run it in mock mode.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use iso_ai::{AiEngine, AiSettings, SidecarConfig, SidecarProvider};
use iso_core::ipc::*;
use iso_engine::{AiLayer, Engine};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, CmdResult, blocking, err};

#[derive(Default)]
pub struct AiState {
    inner: Mutex<Option<(Arc<SidecarProvider>, Arc<AiEngine>)>>,
    device: Mutex<Option<DeviceCode>>,
    next_job: AtomicU64,
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

fn status(state: &State<'_, AppState>, message: Option<String>) -> AiStatus {
    let enabled = state.engine.read().as_ref().is_some_and(|e| e.ai_enabled());
    let mut s = AiStatus {
        sidecar: SidecarState::Off,
        signed_in: false,
        user: None,
        models: vec![],
        enabled_for_repo: enabled,
        mock: mock_mode(),
        device_code: state.ai.device.lock().clone(),
        message,
    };
    if !enabled {
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
pub async fn ai_sign_in(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    let (provider, _) = state.ai.get().map_err(err)?;
    let client = std::env::var("ISOLINE_GITHUB_CLIENT_ID").ok();
    match provider.sign_in(client.as_deref()) {
        Ok(code) => {
            *state.ai.device.lock() = Some(DeviceCode {
                user_code: code.user_code,
                verification_uri: code.verification_uri,
            });
            // Poll in the background; the UI refreshes ai_status while the code is shown.
            let p = Arc::clone(&provider);
            std::thread::spawn(move || {
                for _ in 0..180 {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if matches!(p.sign_in_poll(), Ok((true, _)) | Err(_)) {
                        break;
                    }
                }
            });
            Ok(status(
                &state,
                Some("Enter the code on GitHub to finish signing in.".into()),
            ))
        }
        Err(e) => Ok(status(&state, Some(format!("{e}")))),
    }
}

#[tauri::command]
pub async fn ai_sign_out(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    if let Ok((p, _)) = state.ai.get() {
        let _ = p.sign_out();
    }
    *state.ai.device.lock() = None;
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
    });
    Ok(AiRunHandle { job_id, estimate })
}
