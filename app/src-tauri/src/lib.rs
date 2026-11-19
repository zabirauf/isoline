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
    pub(crate) engine: RwLock<Option<Arc<Engine>>>,
    pub(crate) ai: ai::AiState,
}

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

pub(crate) fn engine(state: &State<'_, AppState>) -> CmdResult<Arc<Engine>> {
    state
        .engine
        .read()
        .clone()
        .ok_or_else(|| "No repo is open.".to_string())
}

/// Run blocking engine work off the UI thread.
pub(crate) async fn blocking<T: Send + 'static>(
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
    eng.set_sink(Some(Arc::new(move |e: Event| {
        let _ = handle.emit(e.name(), e.payload());
    })));
    *state.engine.write() = Some(Arc::clone(&eng));
    ai::attach(&app, &eng);
    // AI status depends on the repo (on or off for it); starting the sidecar can take a
    // second, so the new status comes as an event.
    let h = app.clone();
    std::thread::spawn(move || {
        let st = ai::status(&h.state::<AppState>(), None);
        let _ = h.emit("ai_status", st);
    });
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
async fn get_tree_packed(state: State<'_, AppState>) -> CmdResult<TreePacked> {
    let e = engine(&state)?;
    blocking(move || e.tree_packed()).await
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
async fn get_architecture(state: State<'_, AppState>) -> CmdResult<Option<Architecture>> {
    let e = engine(&state)?;
    blocking(move || e.architecture()).await
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
async fn git_status(state: State<'_, AppState>) -> CmdResult<GitStatus> {
    let e = engine(&state)?;
    blocking(move || e.git_status()).await
}

#[tauri::command]
async fn git_refs(state: State<'_, AppState>) -> CmdResult<GitRefs> {
    let e = engine(&state)?;
    blocking(move || e.git_refs()).await
}

#[tauri::command]
async fn git_switch(
    state: State<'_, AppState>,
    name: String,
    create: bool,
) -> CmdResult<GitStatus> {
    let e = engine(&state)?;
    blocking(move || e.git_switch(&name, create)).await
}

#[tauri::command]
async fn git_fetch(state: State<'_, AppState>) -> CmdResult<GitStatus> {
    let e = engine(&state)?;
    blocking(move || e.git_fetch()).await
}

#[tauri::command]
async fn git_prs(state: State<'_, AppState>) -> CmdResult<PullRequests> {
    let e = engine(&state)?;
    blocking(move || Ok(e.git_prs())).await
}

#[tauri::command]
async fn compare(
    state: State<'_, AppState>,
    base: String,
    head: String,
    three_dot: bool,
    pr: Option<u32>,
) -> CmdResult<Comparison> {
    let e = engine(&state)?;
    blocking(move || e.compare(&base, &head, three_dot, pr)).await
}

#[tauri::command]
async fn compare_file(
    state: State<'_, AppState>,
    base: String,
    head: String,
    path: String,
    old_path: Option<String>,
) -> CmdResult<FileDiffText> {
    let e = engine(&state)?;
    blocking(move || e.compare_file(&base, &head, &path, old_path.as_deref())).await
}

/// Repo (and optionally a file in it) given on the command line: `isoline <repo> [file]`.
#[tauri::command]
async fn launch_args() -> CmdResult<serde_json::Value> {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let repo = args.first().map(|p| {
        dunce::canonicalize(p)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.clone())
    });
    Ok(serde_json::json!({ "repo": repo, "open": args.get(1) }))
}

/// Opens a GitHub page (a pull request, the sign-in page) in the default browser. The
/// webview does not follow target="_blank" links, and only github.com pages are allowed.
#[tauri::command]
async fn open_url(app: tauri::AppHandle, url: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !url.starts_with("https://github.com/") {
        return Err(format!("Only github.com links open from Isoline: {url}"));
    }
    app.opener().open_url(&url, None::<&str>).map_err(err)
}

#[tauri::command]
async fn recent_repos() -> CmdResult<Vec<String>> {
    Ok(iso_engine::recent_repos(&iso_engine::data_dir()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
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
            get_architecture,
            get_tree_packed,
            search,
            get_jobs,
            rename_module,
            recent_repos,
            open_url,
            launch_args,
            git_status,
            git_refs,
            git_switch,
            git_fetch,
            git_prs,
            compare,
            compare_file,
            ai::ai_status,
            ai::ai_sign_in,
            ai::ai_sign_in_cancel,
            ai::ai_sign_out,
            ai::ai_enable,
            ai::ai_estimate,
            ai::ai_run,
            ai::ai_review,
            ai::ai_review_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Isoline");
}
