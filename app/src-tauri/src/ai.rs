//! AI commands. Wired to iso-ai once workstreams F and I land; until then they report that the
//! sidecar is off and never make a network call.

use std::sync::Arc;

use iso_core::ipc::*;
use iso_engine::Engine;
use tauri::{AppHandle, State};

use crate::{AppState, CmdResult};

#[derive(Default)]
pub struct AiState {}

pub(crate) fn attach(_app: &AppHandle, _eng: &Arc<Engine>) {}

fn off(state: &State<'_, AppState>) -> AiStatus {
    let enabled = state.engine.read().as_ref().is_some_and(|e| e.ai_enabled());
    AiStatus {
        sidecar: SidecarState::Off,
        signed_in: false,
        user: None,
        models: vec![],
        enabled_for_repo: enabled,
        mock: false,
        device_code: None,
        message: Some("AI summaries are not available in this build.".into()),
    }
}

#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    Ok(off(&state))
}

#[tauri::command]
pub async fn ai_sign_in(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    Ok(off(&state))
}

#[tauri::command]
pub async fn ai_sign_out(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    Ok(off(&state))
}

#[tauri::command]
pub async fn ai_enable(state: State<'_, AppState>, enabled: bool) -> CmdResult<AiStatus> {
    if let Some(e) = state.engine.read().as_ref() {
        e.set_ai_enabled(enabled).map_err(crate::err)?;
    }
    Ok(off(&state))
}

#[tauri::command]
pub async fn ai_estimate(_scope: AiScope, _path: Option<String>) -> CmdResult<AiEstimate> {
    Err("AI summaries are not available in this build.".into())
}

#[tauri::command]
pub async fn ai_run(_scope: AiScope, _path: Option<String>, _budget_usd: f64) -> CmdResult<AiRunHandle> {
    Err("AI summaries are not available in this build.".into())
}
