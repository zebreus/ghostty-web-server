//! ghostty-web-server (Rust port).
//!
//! Single binary serving:
//!   - GET /                    → embedded client index.html (Yew app shell)
//!   - GET /client.js           → embedded compiled WASM-bindgen JS loader
//!   - GET /client_bg.wasm      → embedded Yew WASM bundle
//!   - GET /dist/ghostty-web.js → vendored ghostty-web JS (terminal renderer)
//!   - GET /ghostty-vt.wasm     → vendored ghostty-web WASM
//!   - GET /favicon.ico         → embedded favicon
//!   - GET /dist/*              → empty-module fallback (matches ghostty-web's
//!                                stale Vite stub references)
//!   - GET /api/sessions        → JSON list of live sessions
//!   - GET /ws                  → WebSocket carrying the JSON wire protocol
//!
//! HTTP and WebSocket share one port (env `PORT`, default 8080) so any
//! reverse proxy (ngrok, nginx) Just Works.

mod active_process;
mod protocol;
mod session;

use std::time::Duration;

use actix_web::middleware::Logger;
use actix_web::web::{self, Bytes};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, Responder};
use futures_util::StreamExt;
use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::protocol::{ClientMsg, ServerMsg};
use crate::session::{AttachResult, Sessions};

/// Embedded client build output (produced by `trunk build`). The Cargo
/// build inlines whatever exists in `client/dist` at compile time. The
/// `cfg` ensures we still compile (with empty assets) before trunk has run,
/// e.g. on first `cargo check` of the workspace.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../client/dist"]
struct ClientAssets;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/assets"]
struct ServerAssets;

const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WS_PING_INTERVAL: Duration = Duration::from_secs(15);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,actix_server::builder=warn");
    }
    env_logger::init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let sessions = Sessions::new();
    session::install_runtime_handle(tokio::runtime::Handle::current());
    session::install_public_sessions(sessions.clone());

    log::info!("ghostty-web-server → http://localhost:{port}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(sessions.clone()))
            .wrap(Logger::default())
            .route("/", web::get().to(index))
            .route("/favicon.ico", web::get().to(favicon))
            .route("/client.js", web::get().to(client_js))
            .route("/client_bg.wasm", web::get().to(client_wasm))
            .route("/dist/ghostty-web.js", web::get().to(ghostty_web_js))
            .route("/ghostty-vt.wasm", web::get().to(ghostty_vt_wasm))
            .route("/dist/{tail:.*}", web::get().to(dist_fallback))
            .route("/snippets/{tail:.*}", web::get().to(snippet_asset))
            .route("/api/sessions", web::get().to(api_sessions))
            .route("/ws", web::get().to(ws_handler))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

// ---------------------------------------------------------------------------
// Static asset routes.
// ---------------------------------------------------------------------------

fn embedded_response<A: RustEmbed>(path: &str, content_type: &str) -> HttpResponse {
    match A::get(path) {
        Some(f) => HttpResponse::Ok()
            .content_type(content_type)
            .body(Bytes::copy_from_slice(f.data.as_ref())),
        None => HttpResponse::NotFound().body(format!("missing asset: {path}")),
    }
}

async fn index() -> impl Responder {
    embedded_response::<ClientAssets>("index.html", "text/html; charset=utf-8")
}

async fn client_js() -> impl Responder {
    embedded_response::<ClientAssets>("client.js", "text/javascript; charset=utf-8")
}

async fn client_wasm() -> impl Responder {
    embedded_response::<ClientAssets>("client_bg.wasm", "application/wasm")
}

async fn favicon() -> impl Responder {
    embedded_response::<ServerAssets>("favicon.ico", "image/vnd.microsoft.icon")
}

async fn ghostty_web_js() -> impl Responder {
    embedded_response::<ServerAssets>("vendor/ghostty-web.js", "text/javascript; charset=utf-8")
}

async fn ghostty_vt_wasm() -> impl Responder {
    embedded_response::<ServerAssets>("vendor/ghostty-vt.wasm", "application/wasm")
}

/// Empty-module fallback for stale Vite "__vite-browser-external-*.js" stubs
/// that ghostty-web's published bundle occasionally references. Same
/// behaviour as the original Elysia route.
async fn dist_fallback() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/javascript; charset=utf-8")
        .body("export {};")
}

/// Serve trunk's wasm-bindgen JS-snippets directory (small inline shims that
/// `client.js` imports relatively, e.g. `dyn_import`).
async fn snippet_asset(path: web::Path<String>) -> impl Responder {
    let rel = format!("snippets/{}", path.into_inner());
    let mime = mime_guess::from_path(&rel)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    embedded_response::<ClientAssets>(&rel, &mime)
}

// ---------------------------------------------------------------------------
// /api/sessions
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct ApiSession {
    id: String,
    #[serde(rename = "startedAt")]
    started_at: u128,
    attached: bool,
    #[serde(rename = "activeProcess")]
    active_process: String,
}

#[derive(serde::Serialize)]
struct ApiSessions {
    sessions: Vec<ApiSession>,
}

async fn api_sessions(sessions: web::Data<Sessions>) -> impl Responder {
    let snap = sessions.snapshot().await;
    let mut out = Vec::with_capacity(snap.len());
    for s in snap {
        let attached = s.attached.lock().await.is_some();
        let ap = active_process::active_process(s.pid).await;
        out.push(ApiSession {
            id: s.id.clone(),
            started_at: s.started_at,
            attached,
            active_process: ap,
        });
    }
    HttpResponse::Ok().json(ApiSessions { sessions: out })
}

// ---------------------------------------------------------------------------
// WebSocket handler.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WsQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    cols: u16,
    rows: u16,
    #[serde(default)]
    take: Option<String>,
}

async fn ws_handler(
    req: HttpRequest,
    body: web::Payload,
    query: web::Query<WsQuery>,
    sessions: web::Data<Sessions>,
) -> Result<HttpResponse, actix_web::Error> {
    let (response, mut session_ws, mut msg_stream) = actix_ws::handle(&req, body)?;

    let cols = query.cols.max(1);
    let rows = query.rows.max(1);
    let take = query.take.as_deref() == Some("1");
    let session_id = query.session_id.clone();
    let sessions = sessions.into_inner();

    // Outbound channel: PTY reader / control code paths push ServerMsg here;
    // a dedicated task forwards them to the WS as JSON text frames.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();

    let attach = sessions.attach(&session_id, cols, rows, take, tx.clone()).await;
    let (session_arc, attach_key) = match attach {
        AttachResult::Busy => {
            // 4002 — "session-busy". Same close code as the original.
            let _ = session_ws.close(Some(actix_ws::CloseReason {
                code: actix_ws::CloseCode::Other(4002),
                description: Some("session-busy".to_string()),
            })).await;
            return Ok(response);
        }
        AttachResult::Attached { outcome, scrollback } => {
            // Replay scrollback (if reattach), then send the initial ack so
            // the client knows its requested size has been honored.
            if let Some(sb) = scrollback {
                let _ = tx.send(ServerMsg::Data { value: sb });
            }
            let _ = tx.send(ServerMsg::Ack { cols, rows });
            (outcome.session, outcome.attach_key)
        }
    };

    // Outbound forwarder: drains the mpsc and writes JSON text frames. Also
    // sends periodic pings so dead clients are reaped within ~WS_IDLE_TIMEOUT.
    let mut session_ws_tx = session_ws.clone();
    actix_web::rt::spawn(async move {
        let mut ping = tokio::time::interval(WS_PING_INTERVAL);
        ping.tick().await; // burn the immediate first tick
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(m) => {
                            let json = serde_json::to_string(&m).unwrap_or_default();
                            if session_ws_tx.text(json).await.is_err() { break; }
                        }
                        None => break, // channel closed: detached / take-over.
                    }
                }
                _ = ping.tick() => {
                    if session_ws_tx.ping(b"").await.is_err() { break; }
                }
            }
        }
        // Best-effort close on the way out.
        let _ = session_ws_tx.close(None).await;
    });

    // Inbound: read frames, dispatch to PTY. Idle timeout matches the
    // original Elysia config (30s).
    let session_for_in = session_arc.clone();
    let session_id_for_in = session_id.clone();
    let sessions_for_in = sessions.clone();
    actix_web::rt::spawn(async move {
        loop {
            let next = tokio::time::timeout(WS_IDLE_TIMEOUT, msg_stream.next()).await;
            let msg = match next {
                Err(_) => break, // idle timeout
                Ok(None) => break,
                Ok(Some(Err(_))) => break,
                Ok(Some(Ok(m))) => m,
            };
            match msg {
                actix_ws::Message::Text(text) => {
                    let Ok(parsed) = serde_json::from_str::<ClientMsg>(&text) else {
                        continue;
                    };
                    match parsed {
                        ClientMsg::Input { value } => {
                            let _ = session_for_in.write_input(value.as_bytes()).await;
                        }
                        ClientMsg::Resize { cols, rows } => {
                            let _ = session_for_in.resize(cols, rows);
                            // ACK so the client only resizes its local
                            // canvas after bash has been told about it.
                            session_for_in
                                .send_to_attached(ServerMsg::Ack { cols, rows })
                                .await;
                        }
                    }
                }
                actix_ws::Message::Ping(p) => {
                    let _ = session_ws.pong(&p).await;
                }
                actix_ws::Message::Close(_) => break,
                _ => {}
            }
        }
        // Detach if we're still the current attachment for this session.
        sessions_for_in
            .detach_if_current(&session_id_for_in, attach_key)
            .await;
    });

    Ok(response)
}
