//! Terminal commands: thin wrappers over `iso_term::Terminals`. Frames (native engine) or
//! raw bytes (Linux) go to the webview over a Tauri channel; a shell that exits sends
//! `term_exit`.

use std::path::PathBuf;

use iso_term::{OpenOptions, Out, TermInfo};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, CmdResult, err};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TermExit {
    id: u32,
    code: Option<u32>,
}

/// True when output is parsed natively and arrives as frames; false when it arrives as raw
/// bytes for ghostty-web.
#[tauri::command]
pub async fn term_native() -> CmdResult<bool> {
    Ok(iso_term::NATIVE)
}

/// Starts a shell in `cwd` (the repo folder; the home folder when there is none).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn term_open(
    app: AppHandle,
    state: State<'_, AppState>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    cell_w: Option<u32>,
    cell_h: Option<u32>,
    theme: Option<serde_json::Value>,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<TermInfo> {
    let opts = OpenOptions {
        cwd: cwd.map(PathBuf::from),
        cols,
        rows,
        cell_w: cell_w.unwrap_or(8),
        cell_h: cell_h.unwrap_or(16),
        theme,
    };
    state
        .term
        .open(
            opts,
            move |o| {
                on_data
                    .send(match o {
                        Out::Frame(j) => InvokeResponseBody::Json(j),
                        Out::Bytes(b) => InvokeResponseBody::Raw(b),
                    })
                    .is_ok()
            },
            move |id, code| {
                let _ = app.emit("term_exit", TermExit { id, code });
            },
        )
        .map_err(err)
}

#[tauri::command]
pub async fn term_input(
    state: State<'_, AppState>,
    id: u32,
    input: serde_json::Value,
) -> CmdResult<()> {
    state.term.input(id, input).map_err(err)
}

#[tauri::command]
pub async fn term_copy(state: State<'_, AppState>, id: u32) -> CmdResult<Option<String>> {
    let st = std::sync::Arc::clone(&state.term);
    crate::blocking(move || st.copy(id)).await
}

#[tauri::command]
pub async fn term_write(state: State<'_, AppState>, id: u32, data: String) -> CmdResult<()> {
    state.term.write(id, &data).map_err(err)
}

#[tauri::command]
pub async fn term_resize(
    state: State<'_, AppState>,
    id: u32,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    state.term.resize(id, cols, rows).map_err(err)
}

#[tauri::command]
pub async fn term_close(state: State<'_, AppState>, id: u32) -> CmdResult<()> {
    state.term.close(id);
    Ok(())
}
