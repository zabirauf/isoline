//! Tauri shell: thin async wrappers over `iso-engine`. Heavy work runs on blocking threads so
//! the UI thread never waits on parsing or SQLite.

use std::sync::Arc;

use iso_core::ipc::*;
use iso_core::{FlowUnit, SymbolId};
use iso_engine::{Engine, Event};
use parking_lot::RwLock;
use tauri::{AppHandle, Emitter, Manager, State};

mod ai;

#[derive(Default)]
pub struct AppState {
    engine: RwLock<Option<Arc<Engine>>>,
    pub(crate) ai: ai::AiState,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn engine(state: &State<'_, AppState>) -> CmdResult<Arc<Engine>> {
    state
        .engine
        .read()
        .clone()
        .ok_or_else(|| "No repo is open.".to_string())
}

/// Run blocking engine work off the UI thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
async fn open_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<RepoSummary> {
    if let Some(old) = state.engine.write().take() {
        old.stop_watch();
        old.set_sink(None);
    }
    let eng = blocking(move || Engine::open(std::path::Path::new(&path))).await?;
    let handle = app.clone();
    eng.set_sink(Some(Arc::new(move |e| {
        let _ = match e {
            Event::IndexProgress(p) => handle.emit("index_progress", p),
            Event::IndexDone(s) => handle.emit("index_done", s),
            Event::FilesChanged(paths) => handle.emit("files_changed", paths),
            Event::JobProgress(j) => handle.emit("job_progress", j),
        };
    })));
    *state.engine.write() = Some(Arc::clone(&eng));
    ai::attach(&app, &eng);
    let bg = Arc::clone(&eng);
    std::thread::Builder::new()
        .name("iso-index".into())
        .spawn(move || {
            if let Err(e) = bg.index() {
                eprintln!("index failed: {e:#}");
            }
            if let Err(e) = bg.start_watch() {
                eprintln!("watch failed: {e:#}");
            }
        })
        .map_err(err)?;
    let e2 = Arc::clone(&eng);
    blocking(move || e2.summary()).await
}

#[tauri::command]
async fn repo_summary(state: State<'_, AppState>) -> CmdResult<RepoSummary> {
    let e = engine(&state)?;
    blocking(move || e.summary()).await
}

#[tauri::command]
async fn get_tree(state: State<'_, AppState>) -> CmdResult<TreeNode> {
    let e = engine(&state)?;
    blocking(move || e.tree()).await
}

#[tauri::command]
async fn get_file_view(state: State<'_, AppState>, path: String) -> CmdResult<FileView> {
    let e = engine(&state)?;
    blocking(move || e.file_view(&path)).await
}

#[tauri::command]
async fn get_flow(state: State<'_, AppState>, path: String, unit: String) -> CmdResult<FlowUnit> {
    let e = engine(&state)?;
    blocking(move || e.flow(&path, &unit)).await
}

#[tauri::command]
async fn get_source(
    state: State<'_, AppState>,
    path: String,
    start: Option<u32>,
    end: Option<u32>,
) -> CmdResult<SourceSlice> {
    let e = engine(&state)?;
    blocking(move || e.source(&path, start, end)).await
}

#[tauri::command]
async fn get_impact(
    state: State<'_, AppState>,
    symbol: String,
    transitive: bool,
) -> CmdResult<Impact> {
    let e = engine(&state)?;
    let id = SymbolId::from_hex(&symbol).ok_or("Bad symbol id.")?;
    blocking(move || e.impact(id, transitive)).await
}

#[tauri::command]
async fn get_module_map(state: State<'_, AppState>) -> CmdResult<ModuleMap> {
    let e = engine(&state)?;
    blocking(move || e.module_map()).await
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> CmdResult<Vec<SearchHit>> {
    let e = engine(&state)?;
    blocking(move || e.search(&query, limit.unwrap_or(30))).await
}

#[tauri::command]
async fn get_jobs(state: State<'_, AppState>) -> CmdResult<JobsState> {
    let e = engine(&state)?;
    blocking(move || Ok(e.jobs_state())).await
}

#[tauri::command]
async fn rename_module(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    color: Option<String>,
) -> CmdResult<ModuleInfo> {
    let e = engine(&state)?;
    blocking(move || e.rename_module(&id, name.as_deref(), color.as_deref())).await
}

#[tauri::command]
async fn recent_repos() -> CmdResult<Vec<String>> {
    Ok(iso_engine::recent_repos(&iso_engine::data_dir()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Open a repo given on the command line: `isoline /path/to/repo`.
            if let Some(path) = std::env::args().nth(1).filter(|a| !a.starts_with('-')) {
                let _ = app.emit("open_requested", path);
            }
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_repo,
            repo_summary,
            get_tree,
            get_file_view,
            get_flow,
            get_source,
            get_impact,
            get_module_map,
            search,
            get_jobs,
            rename_module,
            recent_repos,
            ai::ai_status,
            ai::ai_sign_in,
            ai::ai_sign_out,
            ai::ai_enable,
            ai::ai_estimate,
            ai::ai_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Isoline");
}
