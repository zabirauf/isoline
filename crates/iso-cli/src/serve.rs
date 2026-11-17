//! `iso-cli serve`: a development and test bridge. Serves the built UI and the IPC commands
//! over HTTP on 127.0.0.1 only, so a plain browser or Playwright can drive the real engine
//! (`http://127.0.0.1:<port>/?backend=http`). The desktop app never uses it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Sender, unbounded};
use iso_core::SymbolId;
use iso_engine::{Engine, Event};
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};

#[derive(clap::Args, Clone, Debug)]
pub struct ServeArgs {
    #[arg(long, default_value_t = 7777)]
    pub port: u16,
    /// Built UI folder (app/ui/dist).
    #[arg(long, default_value = "app/ui/dist")]
    pub ui: PathBuf,
}

struct State {
    data: Option<PathBuf>,
    engine: RwLock<Option<Arc<Engine>>>,
    clients: Arc<Mutex<Vec<Sender<String>>>>,
}

pub fn run(data: Option<&Path>, args: ServeArgs) -> Result<()> {
    let addr = format!("127.0.0.1:{}", args.port);
    let server = tiny_http::Server::http(&addr).map_err(|e| anyhow!("bind {addr}: {e}"))?;
    eprintln!(
        "serving http://{addr}/?backend=http (UI from {})",
        args.ui.display()
    );
    let st = Arc::new(State {
        data: data.map(Path::to_path_buf),
        engine: RwLock::new(None),
        clients: Arc::new(Mutex::new(Vec::new())),
    });
    for req in server.incoming_requests() {
        let st = Arc::clone(&st);
        let ui = args.ui.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(&st, &ui, req) {
                eprintln!("request failed: {e:#}");
            }
        });
    }
    Ok(())
}

fn json_response(status: u16, v: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_data(serde_json::to_vec(v).unwrap_or_default())
        .with_status_code(status)
        .with_header(
            "content-type: application/json"
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
}

fn handle(st: &Arc<State>, ui: &Path, mut req: tiny_http::Request) -> Result<()> {
    let url = req.url().split('?').next().unwrap_or("/").to_string();
    if let Some(cmd) = url.strip_prefix("/invoke/") {
        let mut body = String::new();
        req.as_reader().read_to_string(&mut body)?;
        let args: Value = if body.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&body)?
        };
        let resp = match dispatch(st, cmd, &args) {
            Ok(v) => json_response(200, &v),
            Err(e) => json_response(400, &json!({ "error": format!("{e:#}") })),
        };
        req.respond(resp)?;
        return Ok(());
    }
    if url == "/events" {
        // Server-sent events, written straight to the socket and flushed per event: a
        // tiny_http response body goes through an 8 KB chunk buffer, which held back the last
        // events of an index run (the "done" ones) until more traffic pushed them out.
        let (tx, rx) = unbounded::<String>();
        st.clients.lock().push(tx);
        let mut w = req.into_writer();
        w.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: keep-alive\r\n\r\n",
        )?;
        w.flush()?;
        while let Ok(msg) = rx.recv() {
            if w.write_all(format!("data: {msg}\n\n").as_bytes()).is_err() || w.flush().is_err() {
                break;
            }
        }
        return Ok(());
    }
    // Static UI.
    let rel = if url == "/" {
        "index.html".to_string()
    } else {
        url.trim_start_matches('/').to_string()
    };
    if rel.contains("..") {
        req.respond(tiny_http::Response::from_string("bad path").with_status_code(400))?;
        return Ok(());
    }
    let path = ui.join(&rel);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                "html" => "text/html; charset=utf-8",
                "js" | "mjs" => "text/javascript",
                "css" => "text/css",
                "woff2" => "font/woff2",
                "woff" => "font/woff",
                "ttf" => "font/ttf",
                "svg" => "image/svg+xml",
                "png" => "image/png",
                "json" => "application/json",
                _ => "application/octet-stream",
            };
            req.respond(
                tiny_http::Response::from_data(bytes).with_header(
                    format!("content-type: {ct}")
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                ),
            )?;
        }
        Err(_) => {
            req.respond(tiny_http::Response::from_string("not found").with_status_code(404))?
        }
    }
    Ok(())
}

fn arg<'a>(args: &'a Value, k: &str) -> Option<&'a Value> {
    args.get(k).filter(|v| !v.is_null())
}
fn s(args: &Value, k: &str) -> Result<String> {
    arg(args, k)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .with_context(|| format!("missing {k}"))
}
fn u(args: &Value, k: &str) -> Option<u32> {
    arg(args, k).and_then(|v| v.as_u64()).map(|x| x as u32)
}

fn engine(st: &State) -> Result<Arc<Engine>> {
    st.engine
        .read()
        .clone()
        .ok_or_else(|| anyhow!("No repo is open."))
}

fn dispatch(st: &Arc<State>, cmd: &str, args: &Value) -> Result<Value> {
    Ok(match cmd {
        "open_repo" => {
            let path = s(args, "path")?;
            if let Some(old) = st.engine.write().take() {
                old.stop_watch();
                old.set_sink(None);
            }
            let eng = crate::open(Path::new(&path), st.data.as_deref())?;
            let clients = Arc::clone(&st.clients);
            eng.set_sink(Some(Arc::new(move |e| {
                let (name, payload) = match e {
                    Event::IndexProgress(p) => ("index_progress", serde_json::to_value(p)),
                    Event::IndexDone(s) => ("index_done", serde_json::to_value(s)),
                    Event::FilesChanged(p) => ("files_changed", serde_json::to_value(p)),
                    Event::JobProgress(j) => ("job_progress", serde_json::to_value(j)),
                };
                let msg =
                    json!({ "event": name, "payload": payload.unwrap_or(Value::Null) }).to_string();
                clients.lock().retain(|c| c.send(msg.clone()).is_ok());
            })));
            *st.engine.write() = Some(Arc::clone(&eng));
            let bg = Arc::clone(&eng);
            std::thread::spawn(move || {
                if let Err(e) = bg.index() {
                    eprintln!("index failed: {e:#}");
                }
                let _ = bg.start_watch();
            });
            serde_json::to_value(eng.summary()?)?
        }
        "repo_summary" => serde_json::to_value(engine(st)?.summary()?)?,
        "get_tree" => serde_json::to_value(engine(st)?.tree()?)?,
        "get_file_view" => serde_json::to_value(engine(st)?.file_view(&s(args, "path")?)?)?,
        "get_flow" => {
            serde_json::to_value(engine(st)?.flow(&s(args, "path")?, &s(args, "unit")?)?)?
        }
        "get_source" => serde_json::to_value(engine(st)?.source(
            &s(args, "path")?,
            u(args, "start"),
            u(args, "end"),
        )?)?,
        "get_impact" => {
            let id = SymbolId::from_hex(&s(args, "symbol")?).context("bad symbol id")?;
            let t = arg(args, "transitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            serde_json::to_value(engine(st)?.impact(id, t)?)?
        }
        "get_module_map" => serde_json::to_value(engine(st)?.module_map()?)?,
        "search" => serde_json::to_value(
            engine(st)?.search(&s(args, "query")?, u(args, "limit").unwrap_or(30) as usize)?,
        )?,
        "get_jobs" => serde_json::to_value(engine(st)?.jobs_state())?,
        "rename_module" => {
            let name = arg(args, "name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let color = arg(args, "color")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            serde_json::to_value(engine(st)?.rename_module(
                &s(args, "id")?,
                name.as_deref(),
                color.as_deref(),
            )?)?
        }
        "launch_args" => json!({ "repo": null, "open": null }),
        "recent_repos" => serde_json::to_value(iso_engine::recent_repos(
            &st.data.clone().unwrap_or_else(iso_engine::data_dir),
        ))?,
        "ai_status" | "ai_sign_in" | "ai_sign_out" | "ai_enable" => json!({
            "sidecar": "off", "signedIn": false, "user": null, "models": [],
            "enabledForRepo": false, "mock": false, "deviceCode": null,
            "message": "AI summaries are not available over the HTTP bridge."
        }),
        other => return Err(anyhow!("unknown command {other}")),
    })
}
