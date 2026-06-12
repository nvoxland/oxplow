//! Headless oxplow backend daemon.
//!
//! Boots the same `Services` + background orchestration as the Tauri
//! desktop shell, then serves the shared `oxplow-rpc` dispatch over
//! HTTP on loopback:
//!
//! - `POST /ipc/:name` — JSON body = the renderer's invoke args
//!   (camelCase keys); response = the tauri-specta result envelope
//!   `{"status":"ok","data":…}` / `{"status":"error","error":…}` so the
//!   frontend's existing unwrap path works unchanged.
//! - `GET /events` — WebSocket multiplexing the oxplow / lsp /
//!   terminal event channels (wired by the streaming workstream).
//!
//! Single-user by design: bind to `127.0.0.1` on the remote box and
//! reach it through `ssh -L <localPort>:127.0.0.1:<port>`. SSH is the
//! auth layer; the daemon itself runs no TLS and no token check (a
//! bearer stub can be added behind a flag for direct-expose later).
//!
//! Library shape (`run_server` + `Daemon`) so integration tests can
//! boot the full stack on an ephemeral port in-process.

use std::net::SocketAddr;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};

/// Shared router state: the dispatch context (booted services + this
/// box's control-plane coordinates, so agent spawn works remotely).
#[derive(Clone)]
pub struct DaemonState {
    pub ctx: oxplow_rpc::RpcContext,
}

/// Handle returned by [`run_server`]: the bound address (useful when
/// binding port 0 in tests) and the join handle for the accept loop.
pub struct Daemon {
    pub bind_addr: SocketAddr,
    pub task: tokio::task::JoinHandle<()>,
}

/// `POST /ipc/:name` — dispatch a command by wire name. The body is
/// the args object the renderer already sends to Tauri's `invoke`
/// (absent/`null` for no-arg commands). Always replies 200 with the
/// tauri-specta envelope; transport-level errors (unreadable body)
/// reply 400. An unknown command name lands as a NOT_FOUND envelope,
/// mirroring what the dispatch registry returns.
async fn ipc_handler(
    State(state): State<DaemonState>,
    AxumPath(name): AxumPath<String>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let args = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    let result = oxplow_rpc::dispatch(&name, args, &state.ctx).await;
    // Single source of truth for the `{status, data|error}` shape — the
    // Tauri path reaches the byte-identical envelope via typedError +
    // the shared IpcError. See oxplow_rpc::envelope.
    let envelope = oxplow_rpc::ipc_envelope(result);
    (StatusCode::OK, Json(envelope)).into_response()
}

/// Liveness probe for tunnels/scripts (the renderer uses `/ipc/ping`).
async fn health() -> &'static str {
    "ok"
}

/// `GET /events` — WebSocket multiplexing the three event channels the
/// Tauri shell bridges natively (`oxplow:event`, `lsp:event`,
/// `terminal:event`). Each frame is `{"channel":"oxplow"|"lsp"|
/// "terminal","payload":<existing event shape>}` — the payloads are
/// the same serialized types `app.emit` sends locally, so the
/// renderer's handlers are transport-agnostic.
async fn events_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| events_stream(socket, state))
}

/// The three `/events` frame keys in [oxplow, lsp, terminal] order,
/// resolved from `oxplow_app::event_channels::FRAMES` by the channel
/// each key demuxes onto.
fn ws_frame_keys() -> [&'static str; 3] {
    use oxplow_app::event_channels as ch;
    let key_for = |channel: &str| -> &'static str {
        ch::FRAMES
            .iter()
            .find(|(_, c)| *c == channel)
            .map(|(k, _)| *k)
            .unwrap_or_else(|| unreachable!("channel {channel} missing from FRAMES"))
    };
    [key_for(ch::OXPLOW), key_for(ch::LSP), key_for(ch::TERMINAL)]
}

/// Spawn a forwarder per broadcast source into one mpsc, then pump the
/// socket from it. A lagged subscriber just drops frames — the
/// renderer's coarse "bucket changed, refetch" model recovers on the
/// next event (and refetches everything on reconnect anyway).
async fn events_stream(socket: WebSocket, state: DaemonState) {
    let (mut sink, mut inbound) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);

    fn forward<T, F>(
        mut rx: tokio::sync::broadcast::Receiver<T>,
        tx: tokio::sync::mpsc::Sender<String>,
        frame: F,
    ) -> tokio::task::JoinHandle<()>
    where
        T: Clone + Send + 'static,
        F: Fn(&T) -> Option<String> + Send + 'static,
    {
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Some(text) = frame(&event) {
                            if tx.send(text).await.is_err() {
                                break; // client gone
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "events ws forwarder lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }

    fn frame_json<T: serde::Serialize>(channel: &str, event: &T) -> Option<String> {
        match serde_json::to_value(event) {
            Ok(payload) => {
                Some(serde_json::json!({ "channel": channel, "payload": payload }).to_string())
            }
            Err(e) => {
                tracing::warn!(error = %e, channel, "events ws: serialize failed");
                None
            }
        }
    }

    // Frame keys come from the shared channel registry so the daemon,
    // the Tauri shell, and the renderer's demux table can't drift.
    let [oxplow_key, lsp_key, terminal_key] = ws_frame_keys();
    let forwarders = [
        forward(state.ctx.events.subscribe(), tx.clone(), move |e| {
            frame_json(oxplow_key, e)
        }),
        forward(state.ctx.lsp_sessions.subscribe(), tx.clone(), move |e| {
            frame_json(lsp_key, e)
        }),
        forward(
            state.ctx.terminal_sessions.subscribe(),
            tx.clone(),
            move |e| frame_json(terminal_key, e),
        ),
    ];
    drop(tx);

    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            msg = inbound.next() => match msg {
                // Inbound traffic is only ping/close — commands go over
                // /ipc. None/Close ends the stream.
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => continue,
                Some(Err(_)) => break,
            },
        }
    }
    for f in forwarders {
        f.abort();
    }
}

/// Build the daemon router. Split out so tests and the binary share
/// the exact route table.
pub fn router(state: DaemonState) -> Router {
    // Permissive CORS so the frontend can run in a plain browser
    // (Playwright, remote-dev via a served dist/). The daemon binds
    // loopback only and SSH is the auth layer, so origin checks add
    // nothing here; revisit alongside the bearer-token direct-expose
    // mode.
    Router::new()
        .route("/health", get(health))
        .route("/events", get(events_ws))
        .route("/ipc/{name}", post(ipc_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// Bind `addr` and serve the daemon router on it. Returns the bound
/// address (resolves port 0) and the detached server task.
pub async fn run_server(addr: SocketAddr, state: DaemonState) -> std::io::Result<Daemon> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bind_addr = listener.local_addr()?;
    let app = router(state);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "daemon http server exited");
        }
    });
    Ok(Daemon { bind_addr, task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_app::Services;
    use std::process::Command;
    use std::sync::Arc;

    fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "test"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let svc = Arc::new(Services::in_memory(dir.path()).unwrap());
        (svc, dir)
    }

    fn daemon_state(services: Arc<Services>) -> DaemonState {
        DaemonState {
            ctx: oxplow_rpc::RpcContext {
                services,
                plugin_runtime: Some(oxplow_rpc::PluginRuntime {
                    hook_base_url: "http://127.0.0.1:0/hook".into(),
                    mcp_endpoint_url: "http://127.0.0.1:0/mcp".into(),
                    hook_token: "test-token".into(),
                }),
            },
        }
    }

    #[tokio::test]
    async fn ipc_ping_returns_ok_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::Value::Null)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "ok");
        assert_eq!(resp["data"], "pong");
    }

    #[tokio::test]
    async fn ipc_envelope_is_byte_identical_to_shared_wrapper() {
        // Pins that the daemon route delegates to oxplow_rpc::ipc_envelope
        // rather than hand-rolling the shape — for both the ok and error
        // branches. The Tauri host reaches the same shape via typedError +
        // the shared IpcError, so this is the single source of truth.
        let (svc, _dir) = services();
        let state = daemon_state(svc);
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), state.clone())
            .await
            .unwrap();
        let client = reqwest::Client::new();

        for (name, args) in [
            ("ping", serde_json::Value::Null),
            ("no_such_command", serde_json::json!({})),
        ] {
            let expected = oxplow_rpc::ipc_envelope(
                oxplow_rpc::dispatch(name, args.clone(), &state.ctx).await,
            );
            let live: serde_json::Value = client
                .post(format!("http://{}/ipc/{name}", daemon.bind_addr))
                .json(&args)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(live, expected, "envelope drift for /ipc/{name}");
        }
    }

    #[tokio::test]
    async fn ipc_allows_cross_origin_browser_callers() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let client = reqwest::Client::new();

        // Preflight: browsers send OPTIONS before a cross-origin POST
        // with a JSON content-type.
        let preflight = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("origin", "http://localhost:4173")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "content-type")
            .send()
            .await
            .unwrap();
        assert!(
            preflight
                .headers()
                .contains_key("access-control-allow-origin"),
            "preflight must be CORS-approved, got {:?}",
            preflight.headers()
        );

        // The actual response must carry the header too.
        let resp = client
            .post(&url)
            .header("origin", "http://localhost:4173")
            .json(&serde_json::Value::Null)
            .send()
            .await
            .unwrap();
        assert!(resp.headers().contains_key("access-control-allow-origin"));
    }

    #[tokio::test]
    async fn ipc_list_streams_round_trips() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/list_streams", daemon.bind_addr);
        // No body at all — mirrors the renderer omitting args.
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "ok");
        assert!(resp["data"].is_array());
        // ensure_primary hasn't run (no boot orchestration in this
        // test), so the list may be empty — the envelope shape is the
        // contract under test, not project seeding.
    }

    #[tokio::test]
    async fn ipc_unknown_command_is_error_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/definitely_not_a_command", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "error");
        assert_eq!(resp["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn ipc_bad_args_is_invalid_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/get_task", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "error");
        assert_eq!(resp["error"]["code"], "INVALID");
    }

    #[tokio::test]
    async fn events_ws_streams_oxplow_events() {
        use futures::StreamExt as _;
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // The server-side forwarders subscribe after the upgrade
        // completes, so a single immediate emit can race them — keep
        // emitting until the first frame lands.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                svc.events.emit(oxplow_app::OxplowEvent::StreamsChanged);
                match tokio::time::timeout(std::time::Duration::from_millis(200), ws.next()).await {
                    Ok(Some(Ok(msg))) if msg.is_text() => {
                        return msg.into_text().unwrap().to_string()
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("ws frame within timeout");
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["channel"], "oxplow");
        assert_eq!(v["payload"]["kind"], "streamsChanged");
    }

    #[tokio::test]
    async fn events_ws_streams_lsp_session_events() {
        use futures::StreamExt as _;
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // Same subscribe-race handling as the oxplow-events test above.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                svc.lsp_sessions.emit_event_for_tests(
                    oxplow_app::lsp_sessions::LspSessionEvent::SessionStatus {
                        stream_id: "s-1".into(),
                        language: "rust".into(),
                        status: oxplow_app::lsp_sessions::LspSessionStatus::Crashed,
                        message: Some("boom".into()),
                    },
                );
                match tokio::time::timeout(std::time::Duration::from_millis(200), ws.next()).await {
                    Ok(Some(Ok(msg))) if msg.is_text() => {
                        return msg.into_text().unwrap().to_string()
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("ws frame within timeout");
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["channel"], "lsp");
        assert_eq!(v["payload"]["kind"], "sessionStatus");
        assert_eq!(v["payload"]["status"], "crashed");
        assert_eq!(v["payload"]["streamId"], "s-1");
    }

    #[tokio::test]
    async fn health_route_responds() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/health", daemon.bind_addr);
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert_eq!(body, "ok");
    }
}
