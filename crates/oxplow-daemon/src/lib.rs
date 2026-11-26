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
//!   terminal event channels, and carrying the person's terminal input
//!   the other way, in order.
//!
//! Single-user by design: bind to `127.0.0.1` on the remote box and
//! reach it through `ssh -L <localPort>:127.0.0.1:<port>`.
//!
//! **Only the person's UI may call it**. `/ipc` and `/events`
//! require the per-launch UI token ([`DaemonState::token`]); `/health`
//! doesn't. Loopback alone isn't a boundary: the agents this daemon runs,
//! the sources it sandboxes, and any web page the person visits can all
//! reach 127.0.0.1. The token is handed over on stdin by the supervising
//! shell (or generated and printed for a hand-started daemon), never put
//! in an environment variable or a file, so an agent running as the same
//! user can't read it.
//!
//! Library shape (`run_server` + `Daemon`) so integration tests can
//! boot the full stack on an ephemeral port in-process.

use std::net::SocketAddr;

pub mod components;
mod run;

pub use run::run_main;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, Query, State,
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
    /// The UI token every `/ipc` call (`Authorization: Bearer`) and the
    /// `/events` socket (`?token=`) must carry.
    pub token: String,
    pub ctx: oxplow_rpc::RpcContext,
    /// How the daemon tells a live event socket from a gone one.
    pub liveness: Liveness,
}

/// How the daemon knows a page is still there (`.context/remote-daemon.md`
/// "Liveness"): every `interval` it sends a WebSocket ping (a browser
/// answers it without script, so a throttled hidden page stays alive) and a
/// `beat` frame (the page's watchdog); a socket that answers no ping for
/// `missed` intervals, or whose send does not finish within one, is closed.
#[derive(Debug, Clone, Copy)]
pub struct Liveness {
    pub interval: std::time::Duration,
    pub missed: u32,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            interval: std::time::Duration::from_secs(20),
            missed: 2,
        }
    }
}

/// Equal without an early exit, so response timing can't reveal a prefix.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The token a request presents: the bearer header, or (for the
/// WebSocket, whose headers a browser can't set) the `token` query.
fn presented_token(req: &axum::extract::Request, allow_query: bool) -> Option<String> {
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    header.or_else(|| {
        if !allow_query {
            return None;
        }
        req.uri().query().and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("token="))
                .map(str::to_string)
        })
    })
}

async fn require_token(
    State(state): State<DaemonState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let allow_query = req.uri().path() == "/events";
    match presented_token(&req, allow_query) {
        Some(t) if same_secret(&t, &state.token) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "missing or wrong UI token").into_response(),
    }
}

/// A fresh random UI token (256 bits from the OS generator).
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
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
    headers: axum::http::HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let caller = match caller_from_headers(&headers) {
        Ok(caller) => caller,
        Err(message) => {
            let envelope = oxplow_rpc::ipc_envelope(Err(oxplow_rpc::IpcError::invalid(message)));
            return (StatusCode::OK, Json(envelope)).into_response();
        }
    };
    // A page load another one of its device replaced may ask nothing more.
    if let Some(caller) = &caller {
        if state
            .ctx
            .services
            .client_devices
            .check(&caller.connection)
            .is_err()
        {
            let envelope = oxplow_rpc::ipc_envelope(Err(oxplow_rpc::IpcError::replaced()));
            return (StatusCode::OK, Json(envelope)).into_response();
        }
    }
    let args = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    // A request the UI makes is one a person waits on: its database work
    // isn't held back by background work's (`oxplow_db::interactive`).
    let result = oxplow_db::interactive(oxplow_rpc::dispatch_as(
        &name,
        args,
        &state.ctx,
        caller.as_ref(),
    ))
    .await;
    // Single source of truth for the `{status, data|error}` shape — the
    // Tauri path reaches the byte-identical envelope via typedError +
    // the shared IpcError. See oxplow_rpc::envelope.
    let envelope = oxplow_rpc::ipc_envelope(result);
    (StatusCode::OK, Json(envelope)).into_response()
}

/// Who a request comes from (`.context/remote-daemon.md` "Who is asking"):
/// the `Oxplow-Device` and `Oxplow-Connection` headers, both or neither. A
/// request with neither (a script with the token, a test) has no caller; one
/// header alone, or an id of the wrong shape, is refused. Until a host stands
/// in front of the daemon it trusts them: the UI token already says the
/// request is the person's.
fn caller_from_headers(
    headers: &axum::http::HeaderMap,
) -> Result<Option<oxplow_domain::Caller>, String> {
    let read = |name: &str| -> Result<Option<&str>, String> {
        headers
            .get(name)
            .map(|v| v.to_str().map_err(|_| format!("{name} is not text")))
            .transpose()
    };
    match (read("oxplow-device")?, read("oxplow-connection")?) {
        (None, None) => Ok(None),
        (Some(device), Some(connection)) => Ok(Some(oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::parse(device)
                .map_err(|e| format!("Oxplow-Device: {e}"))?,
            connection: oxplow_domain::ConnectionId::parse(connection)
                .map_err(|e| format!("Oxplow-Connection: {e}"))?,
        })),
        _ => Err("Oxplow-Device and Oxplow-Connection come together".into()),
    }
}

/// Liveness probe for tunnels/scripts (the renderer uses `/ipc/ping`).
async fn health() -> &'static str {
    "ok"
}

/// `GET /events` — WebSocket multiplexing the backend event channels
/// (`oxplow:event`, `lsp:event`, `pty:event`). Each
/// frame is `{"channel":"oxplow"|"lsp"|"terminal","payload":<event>}`
/// with the event's serialized shape, so the renderer's handlers are
/// transport-agnostic. The window writes its terminal input on the same
/// socket (`{"channel":"terminal","uri":…,"message":…}`), so keystrokes
/// reach the PTY in the order they were typed.
///
/// The channels are subscribed here, before the upgrade is answered:
/// the upgrade's callback runs only after the client has its
/// `101`, so a client that writes once its socket opens could otherwise
/// cause an event no forwarder was there to see.
///
/// The socket says who it is in its query (`.context/remote-daemon.md` "Who
/// is asking"): `device` and `connection`, both or neither, with the
/// device's `name` and `kind`. A socket with neither (a test, a script)
/// owns no page runs; one that names them wrongly is refused.
#[derive(serde::Deserialize)]
struct EventClient {
    device: Option<String>,
    connection: Option<String>,
    name: Option<String>,
    kind: Option<String>,
}

impl EventClient {
    fn caller(
        self,
    ) -> Result<Option<(oxplow_domain::DeviceDescriptor, oxplow_domain::ConnectionId)>, String>
    {
        let (device, connection) = match (self.device, self.connection) {
            (None, None) => return Ok(None),
            (Some(device), Some(connection)) => (device, connection),
            _ => return Err("device and connection come together".into()),
        };
        let kind = match self.kind.as_deref() {
            None => oxplow_domain::DeviceKind::Desktop,
            Some(kind) => {
                serde_json::from_value(serde_json::Value::String(kind.into())).map_err(|_| {
                    let got: String = kind.chars().take(80).collect();
                    format!("kind: expected desktop, tablet or phone, got `{got}`")
                })?
            }
        };
        Ok(Some((
            oxplow_domain::DeviceDescriptor {
                id: oxplow_domain::DeviceId::parse(&device).map_err(|e| format!("device: {e}"))?,
                name: oxplow_domain::client::device_name(self.name.as_deref()),
                kind,
            },
            oxplow_domain::ConnectionId::parse(&connection)
                .map_err(|e| format!("connection: {e}"))?,
        )))
    }
}

async fn events_ws(
    State(state): State<DaemonState>,
    Query(client): Query<EventClient>,
    ws: WebSocketUpgrade,
) -> Response {
    let named = match client.caller() {
        Ok(named) => named,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    let forwarders = subscribe(&state);
    let services = state.ctx.services.clone();
    // The page load takes over its device once the upgrade is done: a
    // failed upgrade replaces nothing.
    let liveness = state.liveness;
    ws.on_upgrade(move |mut socket| async move {
        let owner = match named
            .map(|(descriptor, connection)| {
                oxplow_app::client_reports::SocketLease::open(&services, descriptor, connection)
            })
            .transpose()
        {
            Ok(owner) => owner,
            Err(_) => {
                let _ = socket.send(replaced_close()).await;
                return;
            }
        };
        events_stream(socket, forwarders, services, owner, liveness).await
    })
}

/// The frame that feeds a page's watchdog (`transport.ts`): a page that
/// hears nothing for longer than a few beats reconnects.
const BEAT_FRAME: &str = r#"{"channel":"beat"}"#;

/// The frame a page sends while the person uses it (at most once a second):
/// its device is the one used last (`ClientDevices::last_used`).
const USED_FRAME: &str = r#"{"channel":"used"}"#;

/// The close frame that tells a page its page load was replaced.
fn replaced_close() -> Message {
    Message::Close(Some(axum::extract::ws::CloseFrame {
        code: oxplow_app::client_reports::devices::REPLACED_CLOSE_CODE,
        reason: "replaced".into(),
    }))
}

/// The normal close that tells a page to reconnect (it missed a call's
/// answer, so the daemon no longer takes it as a host).
fn closing_close() -> Message {
    Message::Close(Some(axum::extract::ws::CloseFrame {
        code: 1000,
        reason: "reconnect".into(),
    }))
}

/// Resolves once the daemon asks `owner`'s socket to close; never for a
/// socket that names no device.
async fn closing(owner: &Option<oxplow_app::client_reports::SocketLease>) {
    match owner {
        Some(owner) => owner.closing().await,
        None => std::future::pending().await,
    }
}

/// Resolves once `owner`'s page load is replaced; never for a socket that
/// names no device.
async fn replaced(owner: &Option<oxplow_app::client_reports::SocketLease>) {
    match owner {
        Some(owner) => owner.replaced().await,
        None => std::future::pending().await,
    }
}

/// One channel's subscription, waiting for the socket's queue to forward
/// into.
type Forwarder =
    Box<dyn FnOnce(tokio::sync::mpsc::Sender<String>) -> tokio::task::JoinHandle<()> + Send>;

/// A subscription to `rx`, each event framed by `frame`. A lagged
/// subscriber has lost frames, and it says so: the renderer reads everything
/// again, as it does on a reconnect.
fn forward<T, F>(mut rx: tokio::sync::broadcast::Receiver<T>, frame: F) -> Forwarder
where
    T: Clone + Send + 'static,
    F: Fn(&T) -> Option<String> + Send + 'static,
{
    Box::new(move |tx| {
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
                        if tx.send(RESYNC_FRAME.to_string()).await.is_err() {
                            break; // client gone
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    })
}

/// What a client is told when it has lost frames.
const RESYNC_FRAME: &str = r#"{"channel":"resync"}"#;

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

/// Subscribe every channel the socket carries. Frame keys come from the
/// shared channel registry so the daemon, the Tauri shell, and the
/// renderer's demux table can't drift.
fn subscribe(state: &DaemonState) -> [Forwarder; 3] {
    let [oxplow_key, lsp_key, terminal_key] = ws_frame_keys();
    [
        forward(state.ctx.events.subscribe_ui(), move |e| {
            frame_json(oxplow_key, e)
        }),
        forward(state.ctx.lsp_sessions.subscribe(), move |e| {
            frame_json(lsp_key, e)
        }),
        forward(state.ctx.terminal_sessions.subscribe(), move |e| {
            frame_json(terminal_key, e)
        }),
    ]
}

/// The `/events` frame keys in [oxplow, lsp, terminal] order,
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

/// Start each channel's forwarder into one queue, then pump the socket
/// from it.
async fn events_stream(
    socket: WebSocket,
    subscriptions: [Forwarder; 3],
    services: std::sync::Arc<oxplow_app::Services>,
    owner: Option<oxplow_app::client_reports::SocketLease>,
    liveness: Liveness,
) {
    let (mut sink, mut inbound) = socket.split();
    let mut beat = tokio::time::interval(liveness.interval);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    beat.tick().await; // the first tick is now
    let mut last_pong = tokio::time::Instant::now();
    let gone_after = liveness.interval * liveness.missed;
    // A send that does not finish within an interval: the peer is gone.
    macro_rules! send {
        ($msg:expr) => {
            match tokio::time::timeout(liveness.interval, sink.send($msg)).await {
                Ok(Ok(())) => true,
                _ => false,
            }
        };
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let forwarders = subscriptions.map(|start| start(tx.clone()));
    drop(tx);

    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(text) => {
                    if !send!(Message::Text(text.into())) {
                        break;
                    }
                }
                None => break,
            },
            () = replaced(&owner) => {
                let _ = send!(replaced_close());
                break;
            }
            () = closing(&owner) => {
                let _ = send!(closing_close());
                break;
            }
            _ = beat.tick() => {
                if last_pong.elapsed() > gone_after {
                    tracing::info!("event socket: no pong; the page is gone");
                    break;
                }
                if !send!(Message::Ping(Vec::new().into())) || !send!(Message::Text(BEAT_FRAME.into())) {
                    break;
                }
            }
            msg = inbound.next() => match msg {
                // Inbound traffic is terminal input, ping and close —
                // commands go over /ipc. None/Close ends the stream.
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Text(text))) if text.as_str() == USED_FRAME => {
                    if let Some(owner) = &owner {
                        owner.used();
                    }
                }
                Some(Ok(Message::Text(text))) => terminal_frame(&services, &text),
                Some(Ok(Message::Pong(_))) => last_pong = tokio::time::Instant::now(),
                Some(Ok(_)) => continue,
                Some(Err(_)) => break,
            },
        }
    }
    for f in forwarders {
        f.abort();
    }
}

/// One terminal-input frame from the window: its message queued for the
/// terminal's PTY, before the next frame is read. A frame that isn't one,
/// or names no terminal, is logged and dropped.
fn terminal_frame(services: &oxplow_app::Services, text: &str) {
    #[derive(serde::Deserialize)]
    struct Frame {
        channel: String,
        uri: String,
        message: String,
    }
    let [_, _, terminal] = ws_frame_keys();
    match serde_json::from_str::<Frame>(text) {
        Ok(f) if f.channel == terminal => {
            if let Err(e) =
                oxplow_rpc::commands::terminal::terminal_input(services, f.uri.clone(), f.message)
            {
                tracing::warn!(uri = %f.uri, error = %e, "terminal input refused");
            }
        }
        Ok(f) => tracing::warn!(channel = %f.channel, "events ws: no inbound channel"),
        Err(e) => tracing::warn!(error = %e, "events ws: unreadable inbound frame"),
    }
}

/// Build the daemon router. Split out so tests and the binary share
/// the exact route table.
pub fn router(state: DaemonState) -> Router {
    // Permissive CORS so the frontend can run in a plain browser
    // (Playwright, remote-dev via a served dist/). It exposes nothing:
    // every route but /health needs the bearer UI token, which a page
    // can't obtain, and no cookies are involved.
    let guarded = Router::new()
        .route("/events", get(events_ws))
        .route("/ipc/{name}", post(ipc_handler))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_token,
        ));
    // Custom component bundles (P6b.D3): ungated — a sandboxed frame can't
    // carry the token — and outside the CORS layer, so a page can't read
    // them with `fetch`.
    let components = Router::new()
        .route("/components/v/{version}", get(components::component_root))
        .route("/components/v/{version}/", get(components::component_index))
        .route(
            "/components/v/{version}/{*path}",
            get(components::component_file),
        )
        // The client library a bundle loads (P9.A4).
        .route("/component-lib/{file}", get(components::component_lib));
    Router::new()
        .route("/health", get(health))
        .merge(guarded)
        .layer(tower_http::cors::CorsLayer::permissive())
        .merge(components)
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
    #![allow(
        clippy::disallowed_methods,
        reason = "a test seeds the database through its stores"
    )]

    use super::*;
    use oxplow_app::Services;
    use std::process::Command;
    use std::sync::Arc;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
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
        let svc = Services::in_memory(dir.path()).await.unwrap();
        (svc, dir)
    }

    const UI_TOKEN: &str = "test-ui-token";

    /// A client carrying the renderer's token, as every UI request does.
    fn client() -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {UI_TOKEN}").parse().unwrap(),
        );
        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap()
    }

    fn daemon_state(services: Arc<Services>) -> DaemonState {
        DaemonState {
            liveness: Liveness::default(),
            token: UI_TOKEN.into(),
            ctx: oxplow_rpc::RpcContext {
                services,
                plugin_runtime: Some(oxplow_rpc::PluginRuntime {
                    hook_base_url: "http://127.0.0.1:0/hook".into(),
                    mcp_endpoint_url: "http://127.0.0.1:0/mcp".into(),
                    otlp_base_url: "http://127.0.0.1:0".into(),
                }),
            },
        }
    }

    #[tokio::test]
    async fn ipc_and_events_refuse_callers_without_the_ui_token() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let bare = reqwest::Client::new();
        for req in [
            bare.post(&url),
            bare.post(&url).bearer_auth("wrong"),
            bare.post(&url).header("authorization", "test-ui-token"),
        ] {
            let resp = req.send().await.unwrap();
            assert_eq!(resp.status(), 401);
        }
        // The token rides the query only for the WebSocket (browsers can't
        // set its headers); /ipc takes it in the header.
        let resp = bare
            .post(format!("{url}?token={UI_TOKEN}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        assert_eq!(client().post(&url).send().await.unwrap().status(), 200);

        for bad in ["", "?token=wrong"] {
            let ws = format!("ws://{}/events{bad}", daemon.bind_addr);
            assert!(
                tokio_tungstenite::connect_async(&ws).await.is_err(),
                "{bad}"
            );
        }
        // Liveness stays open for tunnels and scripts.
        let health = bare
            .get(format!("http://{}/health", daemon.bind_addr))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);
    }

    /// P6b.D3: a private extension's declared component bundle is served
    /// without the token, with a CSP that lets it load only its own files;
    /// anything else is a 404.
    #[tokio::test]
    async fn component_bundles_are_served_with_their_csp_and_nothing_else() {
        let (svc, dir) = services().await;
        let ext = dir.path().join(".oxplow/extensions/x");
        std::fs::create_dir_all(ext.join("components/c/assets")).unwrap();
        std::fs::create_dir_all(ext.join("pages")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: c }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/c/index.html"), "<!doctype html>hi").unwrap();
        std::fs::write(ext.join("components/c/assets/app.js"), "1").unwrap();
        let version = load(&svc, dir.path(), "x", "c").await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let base = format!("http://{}", daemon.bind_addr);
        let bare = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let folder_url = format!("/components/v/{version}/");
        for path in [folder_url.clone(), format!("{folder_url}index.html")] {
            let resp = bare.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), 200, "{path}");
            let h = resp.headers();
            assert_eq!(h["content-type"], "text/html; charset=utf-8");
            assert_eq!(h["cache-control"], "no-store");
            assert_eq!(h["x-content-type-options"], "nosniff");
            assert_eq!(h["referrer-policy"], "no-referrer");
            assert_eq!(
                h["content-security-policy"].to_str().unwrap(),
                oxplow_app::component_http::bundle_csp(
                    &format!("{base}{folder_url}"),
                    &format!("{base}/component-lib/")
                )
            );
            assert!(
                h.get("access-control-allow-origin").is_none(),
                "outside CORS"
            );
            assert_eq!(resp.text().await.unwrap(), "<!doctype html>hi");
        }
        let js = bare
            .get(format!("{base}{folder_url}assets/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            js.headers()["content-type"],
            "text/javascript; charset=utf-8"
        );
        let folder = bare
            .get(format!("{base}/components/v/{version}"))
            .send()
            .await
            .unwrap();
        assert_eq!(folder.status(), 308);
        assert_eq!(folder.headers()["location"], folder_url.as_str());
        // what's served is the snapshot — the disk changing after
        // the load isn't what the frame gets.
        std::fs::write(
            ext.join("components/c/index.html"),
            "<!doctype html>changed",
        )
        .unwrap();
        let again = bare
            .get(format!("{base}{folder_url}"))
            .send()
            .await
            .unwrap();
        assert_eq!(again.text().await.unwrap(), "<!doctype html>hi");
        for path in [
            "/components/v/0123abcd/".to_string(),
            format!("{folder_url}nope.js"),
            format!("{folder_url}assets"),
            format!("{folder_url}%2e%2e/%2e%2e/extension.yaml"),
            "/components/primary/x/c/".to_string(),
        ] {
            let resp = bare.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), 404, "{path}");
        }
    }

    /// Load `component` of extension `ext` under `root` as a frame's host
    /// does: its version.
    async fn load(
        svc: &Arc<Services>,
        root: &std::path::Path,
        ext: &str,
        component: &str,
    ) -> String {
        // What was written is deployed first, as a person or agent does.
        oxplow_app::extension::extension_deploy::deploy(svc, ext, true)
            .await
            .expect("the extension deploys");
        let ext = svc.extension_catalog.named(root, ext).unwrap();
        let declared = ext
            .custom_components
            .iter()
            .find(|c| c.id == component)
            .unwrap()
            .clone();
        svc.component_bundles
            .load(root, &ext, &declared)
            .unwrap()
            .version
            .clone()
    }

    /// P9.A4: the component client library — one fixed file, served like a
    /// bundle's: ungated, outside CORS, to a loopback `Host` only.
    #[tokio::test]
    async fn the_client_library_is_served_to_loopback_only_as_javascript() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let base = format!("http://{}", daemon.bind_addr);
        let bare = reqwest::Client::new();
        let resp = bare
            .get(format!("{base}/component-lib/oxplow-component.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let h = resp.headers();
        assert_eq!(h["content-type"], "text/javascript; charset=utf-8");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["cache-control"], "no-cache");
        assert!(
            h.get("access-control-allow-origin").is_none(),
            "outside CORS"
        );
        assert!(resp.text().await.unwrap().contains("global.oxplow = "));
        let types = bare
            .get(format!("{base}/component-lib/oxplow-component.d.ts"))
            .send()
            .await
            .unwrap();
        assert_eq!(types.status(), 200);
        // the kit's stylesheet, as CSS.
        let kit = bare
            .get(format!("{base}/component-lib/oxplow-kit.css"))
            .send()
            .await
            .unwrap();
        assert_eq!(kit.status(), 200);
        assert_eq!(kit.headers()["content-type"], "text/css; charset=utf-8");
        assert!(kit.text().await.unwrap().contains(".ox-"));
        for path in [
            "/component-lib/other.js",
            "/component-lib/",
            "/component-lib",
        ] {
            let resp = bare.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(resp.status(), 404, "{path}");
        }
        let rebound = bare
            .get(format!("{base}/component-lib/oxplow-component.js"))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(rebound.status(), 404, "only a loopback Host");
    }

    /// DNS rebinding: a page whose name now resolves to 127.0.0.1 reaches
    /// the daemon with its own `Host`; only a loopback `Host` is served.
    #[tokio::test]
    async fn component_bundles_answer_only_a_loopback_host() {
        let (svc, dir) = services().await;
        let ext = dir.path().join(".oxplow/extensions/x");
        std::fs::create_dir_all(ext.join("components/c")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: c }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/c/index.html"), "hi").unwrap();
        let version = load(&svc, dir.path(), "x", "c").await;
        let folder = format!("/components/v/{version}/");
        let bare_folder = format!("/components/v/{version}");
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let port = daemon.bind_addr.port();
        let bare = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let get = |path: &str, host: String| {
            bare.get(format!("http://{}{path}", daemon.bind_addr))
                .header("host", host)
                .send()
        };
        for host in [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            "localhost".to_string(),
        ] {
            let resp = get(&folder, host.clone()).await.unwrap();
            assert_eq!(resp.status(), 200, "{host}");
            assert_eq!(
                resp.headers()["content-security-policy"].to_str().unwrap(),
                oxplow_app::component_http::bundle_csp(
                    &format!("http://{host}{folder}"),
                    &format!("http://{host}/component-lib/")
                ),
            );
        }
        for host in [
            format!("evil.example:{port}"),
            "evil.example".to_string(),
            format!("127.0.0.1.evil.example:{port}"),
            format!("localhost.evil.example:{port}"),
            String::new(),
        ] {
            for path in [&folder, &bare_folder] {
                let resp = get(path, host.clone()).await.unwrap();
                assert_eq!(resp.status(), 404, "{host} {path}");
            }
        }
    }

    /// A stream's page loads its component from the stream's worktree: a
    /// version of its own, served from what that worktree held.
    fn headers(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    const DEVICE: &str = "dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427";
    const CONNECTION: &str = "con-6fa459ea-ee8a-3ca4-894e-db77e160355e";

    /// A request says who asks with two headers, both or neither: a request
    /// with neither (a script, a test) has no caller.
    #[test]
    fn a_request_names_its_caller_in_two_headers() {
        assert_eq!(caller_from_headers(&headers(&[])), Ok(None));
        let caller = caller_from_headers(&headers(&[
            ("oxplow-device", DEVICE),
            ("oxplow-connection", CONNECTION),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(caller.device.as_str(), DEVICE);
        assert_eq!(caller.connection.as_str(), CONNECTION);
        for bad in [
            vec![("oxplow-device", DEVICE)],
            vec![("oxplow-connection", CONNECTION)],
            vec![
                ("oxplow-device", "dev-x"),
                ("oxplow-connection", CONNECTION),
            ],
            vec![("oxplow-device", CONNECTION), ("oxplow-connection", DEVICE)],
        ] {
            assert!(caller_from_headers(&headers(&bad)).is_err(), "{bad:?}");
        }
    }

    /// A malformed caller is refused before any command runs.
    #[tokio::test]
    async fn ipc_refuses_a_malformed_caller() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let ok: serde_json::Value = client()
            .post(&url)
            .header("Oxplow-Device", DEVICE)
            .header("Oxplow-Connection", CONNECTION)
            .json(&serde_json::Value::Null)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(ok["status"], "ok");
        let refused: serde_json::Value = client()
            .post(&url)
            .header("Oxplow-Device", "dev-not-an-id")
            .header("Oxplow-Connection", CONNECTION)
            .json(&serde_json::Value::Null)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(refused["status"], "error", "{refused}");
        assert!(
            refused.to_string().contains("Oxplow-Device"),
            "the error names the header: {refused}"
        );
    }

    #[tokio::test]
    async fn ipc_ping_returns_ok_envelope() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let resp: serde_json::Value = client()
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
        let (svc, _dir) = services().await;
        let state = daemon_state(svc);
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), state.clone())
            .await
            .unwrap();
        let client = client();

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
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let client = client();

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
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/list_streams", daemon.bind_addr);
        // No body at all — mirrors the renderer omitting args.
        let resp: serde_json::Value = client()
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
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/definitely_not_a_command", daemon.bind_addr);
        let resp: serde_json::Value = client()
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
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/list_threads", daemon.bind_addr);
        let resp: serde_json::Value = client()
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

    /// the socket listens before it opens — an event emitted the
    /// moment a client sees the upgrade reaches it, so a client can open
    /// the socket, then write, and never miss what its write caused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_event_emitted_as_the_socket_opens_arrives() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        for _ in 0..50 {
            let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            svc.events
                .emit(oxplow_app::OxplowEvent::BackgroundTasksChanged);
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .expect("the event emitted as the socket opened")
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(v["payload"]["kind"], "backgroundTasksChanged");
        }
    }

    /// A page's event socket names its device and its page load, with the
    /// device's name and kind; the daemon knows the device while it is
    /// connected, and refuses a socket that names them wrongly.
    #[tokio::test]
    async fn an_event_socket_says_which_device_it_is() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let base = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        let device = oxplow_domain::DeviceId::parse(DEVICE).unwrap();
        let (mut socket, _) = tokio_tungstenite::connect_async(format!(
            "{base}&device={DEVICE}&connection={CONNECTION}&name=Tab%20S9&kind=tablet"
        ))
        .await
        .unwrap();
        let seen = svc
            .client_devices
            .get(&device)
            .expect("the device is connected");
        assert_eq!(seen.name, "Tab S9");
        assert_eq!(seen.kind, oxplow_domain::DeviceKind::Tablet);
        // What the window reports showing is shown while its socket is open.
        let caller = oxplow_domain::Caller {
            device: device.clone(),
            connection: oxplow_domain::ConnectionId::parse(CONNECTION).unwrap(),
        };
        let ctx = oxplow_rpc::RpcContext {
            services: svc.clone(),
            plugin_runtime: None,
        };
        oxplow_rpc::dispatch_as(
            "report_tabs",
            serde_json::json!({ "report": { "focused": true, "visible": true, "shown_thread": null,
                                            "threads": [], "detail_json": null } }),
            &ctx,
            Some(&caller),
        )
        .await
        .unwrap();
        let now = oxplow_domain::Timestamp::now();
        assert_eq!(
            svc.client_reports.live_ids(now),
            vec![CONNECTION.to_string()]
        );
        socket.close(None).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while svc.client_devices.get(&device).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a closed socket's device is no longer connected");
        assert!(
            svc.client_reports
                .live_ids(oxplow_domain::Timestamp::now())
                .is_empty(),
            "a closed window's report is gone with it"
        );

        for bad in [
            format!("{base}&device={DEVICE}"),
            format!("{base}&device=dev-x&connection={CONNECTION}"),
            format!("{base}&device={DEVICE}&connection={CONNECTION}&kind=toaster"),
        ] {
            assert!(
                tokio_tungstenite::connect_async(&bad).await.is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[tokio::test]
    async fn closing_an_event_socket_releases_only_its_windows_page_runs() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const ONE: &str = "con-11111111-1111-4111-8111-111111111111";
        const TWO: &str = "con-22222222-2222-4222-8222-222222222222";
        // Two devices: one device keeps one window per project.
        const OTHER: &str = "dev-2b4e28ba-2fa1-11d2-883f-0016d3cca427";
        let url = |device, connection| {
            format!(
                "ws://{}/events?token={UI_TOKEN}&device={device}&connection={connection}&name=Mac&kind=desktop",
                daemon.bind_addr
            )
        };
        let (mut one, _) = tokio_tungstenite::connect_async(url(DEVICE, ONE))
            .await
            .unwrap();
        let (mut two, _) = tokio_tungstenite::connect_async(url(OTHER, TWO))
            .await
            .unwrap();
        let ctx = oxplow_rpc::RpcContext {
            services: svc.clone(),
            plugin_runtime: None,
        };
        let yaml =
            "title: Open\nloaders: { a: { sql: SELECT 1 AS n } }\nview: { viz: number, from: a }\n";
        let first = oxplow_rpc::commands::pages::open_page(
            &oxplow_rpc::RpcRequest {
                ctx: &ctx,
                caller: Some(&oxplow_domain::Caller {
                    device: oxplow_domain::DeviceId::parse(DEVICE).unwrap(),
                    connection: oxplow_domain::ConnectionId::parse(ONE).unwrap(),
                }),
            },
            None,
            Some(yaml.into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let second = oxplow_rpc::commands::pages::open_page(
            &oxplow_rpc::RpcRequest {
                ctx: &ctx,
                caller: Some(&oxplow_domain::Caller {
                    device: oxplow_domain::DeviceId::parse(OTHER).unwrap(),
                    connection: oxplow_domain::ConnectionId::parse(TWO).unwrap(),
                }),
            },
            None,
            Some(yaml.into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        one.close(None).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while svc.page_runs.get(&first.run_id).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the disconnected window's runs are released");
        assert!(svc.page_runs.get(&second.run_id).is_some());
        two.close(None).await.unwrap();
    }

    /// A page that says it is in use makes its device the one used last.
    #[tokio::test]
    async fn a_page_in_use_is_the_device_used_last() {
        use futures::SinkExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const MAC: &str = "con-11111111-1111-4111-8111-111111111111";
        const TAB: &str = "con-22222222-2222-4222-8222-222222222222";
        const TABLET: &str = "dev-2b4e28ba-2fa1-11d2-883f-0016d3cca427";
        let url = |device, connection| {
            format!(
                "ws://{}/events?token={UI_TOKEN}&device={device}&connection={connection}&name=x&kind=desktop",
                daemon.bind_addr
            )
        };
        let (mut mac, _) = tokio_tungstenite::connect_async(url(DEVICE, MAC))
            .await
            .unwrap();
        let (_tab, _) = tokio_tungstenite::connect_async(url(TABLET, TAB))
            .await
            .unwrap();
        mac.send(tokio_tungstenite::tungstenite::Message::Text(
            USED_FRAME.into(),
        ))
        .await
        .unwrap();
        let mac_device = oxplow_domain::DeviceId::parse(DEVICE).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while svc.client_devices.last_used().map(|(d, _)| d) != Some(mac_device.clone()) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Mac, used, is the device used last");
    }

    /// A socket that names no device cannot make one the device used last.
    #[tokio::test]
    async fn use_from_a_socket_with_no_device_is_ignored() {
        use futures::SinkExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const MAC: &str = "con-11111111-1111-4111-8111-111111111111";
        let (mut anonymous, _) = tokio_tungstenite::connect_async(format!(
            "ws://{}/events?token={UI_TOKEN}",
            daemon.bind_addr
        ))
        .await
        .unwrap();
        let (_mac, _) = tokio_tungstenite::connect_async(format!(
            "ws://{}/events?token={UI_TOKEN}&device={DEVICE}&connection={MAC}&name=Mac&kind=desktop",
            daemon.bind_addr
        ))
        .await
        .unwrap();
        anonymous
            .send(tokio_tungstenite::tungstenite::Message::Text(
                USED_FRAME.into(),
            ))
            .await
            .unwrap();
        // Give the daemon time to hear it; the only device is still the one used last.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let mac = oxplow_domain::DeviceId::parse(DEVICE).unwrap();
        assert_eq!(svc.client_devices.last_used().map(|(d, _)| d), Some(mac));
        assert_eq!(
            svc.client_devices.list().len(),
            1,
            "no device for the anonymous socket"
        );
    }

    /// A page that answers no ping is gone: its socket closes and its device
    /// leaves. One that answers stays, and hears the watchdog's beat.
    #[tokio::test]
    async fn a_socket_whose_page_answers_no_ping_is_closed() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let state = DaemonState {
            // Short, so the test sees a gone peer in moments.
            liveness: Liveness {
                interval: std::time::Duration::from_millis(100),
                missed: 2,
            },
            ..daemon_state(svc.clone())
        };
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), state)
            .await
            .unwrap();
        const LIVE: &str = "con-11111111-1111-4111-8111-111111111111";
        const GONE: &str = "con-22222222-2222-4222-8222-222222222222";
        const OTHER: &str = "dev-2b4e28ba-2fa1-11d2-883f-0016d3cca427";
        let url = |device, connection| {
            format!(
                "ws://{}/events?token={UI_TOKEN}&device={device}&connection={connection}&name=Mac&kind=desktop",
                daemon.bind_addr
            )
        };
        let (mut live, _) = tokio_tungstenite::connect_async(url(DEVICE, LIVE))
            .await
            .unwrap();
        // Never read: tungstenite answers a ping only while it reads.
        let (_gone, _) = tokio_tungstenite::connect_async(url(OTHER, GONE))
            .await
            .unwrap();
        svc.client_host.register(GONE, vec!["tabs.write".into()]);
        let other = oxplow_domain::DeviceId::parse(OTHER).unwrap();
        let mine = oxplow_domain::DeviceId::parse(DEVICE).unwrap();
        let mut beats = 0;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while svc.client_devices.get(&other).is_some() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the silent page is let go"
            );
            if let Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) =
                tokio::time::timeout(std::time::Duration::from_millis(50), live.next()).await
            {
                if text.contains("\"beat\"") {
                    beats += 1;
                }
            }
        }
        assert!(beats > 0, "the live page hears the beat");
        assert!(!svc.client_host.hosts(GONE), "the gone page hosts no call");
        assert!(
            svc.client_devices.get(&mine).is_some(),
            "the page that answers stays"
        );
    }

    /// A window that misses a call's answer is forgotten as a host and its
    /// socket closes (a normal close, not "replaced"): a live page then
    /// reconnects and registers again.
    #[tokio::test]
    async fn a_call_that_times_out_closes_the_windows_socket() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const CON: &str = "con-11111111-1111-4111-8111-111111111111";
        let url = format!(
            "ws://{}/events?token={UI_TOKEN}&device={DEVICE}&connection={CON}&name=Mac&kind=desktop",
            daemon.bind_addr
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let client_host = std::sync::Arc::new(oxplow_app::client_host::ClientHost::with_timeout(
            svc.events.clone(),
            svc.client_devices.clone(),
            std::time::Duration::from_millis(100),
        ));
        client_host.register(CON, vec!["tabs.write".into()]);
        let run = oxplow_domain::run::RunContext::core(
            oxplow_domain::Actor::Agent {
                session_id: None,
                thread_id: Some(oxplow_domain::ThreadId::new(1)),
                stream_id: None,
            },
            oxplow_domain::run::Mode::External,
        );
        client_host
            .call(&run, "tabs.write", "open", serde_json::json!({}))
            .await
            .unwrap_err();
        let closed = loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
                .await
                .expect("the silent window's socket closes");
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Close(close))) = frame {
                break close;
            }
        };
        let code = closed.map(|c| u16::from(c.code));
        assert_ne!(
            code,
            Some(oxplow_app::client_reports::devices::REPLACED_CLOSE_CODE),
            "a normal close: the page reconnects"
        );
    }

    /// A device keeps one page load: a new one takes over, the older one's
    /// socket closes saying it was replaced, and the older one is refused
    /// from then on, on `/ipc` and on a new socket.
    #[tokio::test]
    async fn a_new_page_load_of_a_device_replaces_the_older() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const OLD: &str = "con-11111111-1111-4111-8111-111111111111";
        const NEW: &str = "con-22222222-2222-4222-8222-222222222222";
        let url = |connection| {
            format!(
                "ws://{}/events?token={UI_TOKEN}&device={DEVICE}&connection={connection}&name=Mac&kind=desktop",
                daemon.bind_addr
            )
        };
        let closed_replaced = |frame: Option<
            Result<tokio_tungstenite::tungstenite::Message, _>,
        >| {
            matches!(
                frame,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(close))))
                    if u16::from(close.code) == oxplow_app::client_reports::devices::REPLACED_CLOSE_CODE
            )
        };
        let (mut old, _) = tokio_tungstenite::connect_async(url(OLD)).await.unwrap();
        // The older page load hosts the daemon's calls until it is replaced.
        svc.client_host.register(OLD, vec!["tabs.write".into()]);
        let (_new, _) = tokio_tungstenite::connect_async(url(NEW)).await.unwrap();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), old.next())
            .await
            .expect("the older socket hears it was replaced");
        assert!(
            closed_replaced(frame),
            "the older socket closes as replaced"
        );
        assert!(
            !svc.client_host.hosts(OLD),
            "the replaced page load hosts no more calls"
        );

        let ask = |connection: &'static str| {
            client()
                .post(format!("http://{}/ipc/list_streams", daemon.bind_addr))
                .header("oxplow-device", DEVICE)
                .header("oxplow-connection", connection)
                .json(&serde_json::json!({}))
                .send()
        };
        let refused: serde_json::Value = ask(OLD).await.unwrap().json().await.unwrap();
        assert_eq!(refused["error"]["code"], "REPLACED", "{refused}");
        let answered: serde_json::Value = ask(NEW).await.unwrap().json().await.unwrap();
        assert_eq!(answered["status"], "ok", "{answered}");

        let (mut again, _) = tokio_tungstenite::connect_async(url(OLD)).await.unwrap();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), again.next())
            .await
            .expect("a replaced page load's new socket is refused");
        assert!(closed_replaced(frame), "and never comes back");
    }

    /// A page load keeps its connection across reconnects, so its new socket
    /// can open before the daemon sees the old one close (a tablet that
    /// slept). The old socket's close must leave the live window as it is.
    #[tokio::test]
    async fn a_stale_socket_closing_leaves_the_reconnected_window_connected() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        const ONE: &str = "con-11111111-1111-4111-8111-111111111111";
        let url = format!(
            "ws://{}/events?token={UI_TOKEN}&device={DEVICE}&connection={ONE}&name=Mac&kind=desktop",
            daemon.bind_addr
        );
        let device = oxplow_domain::DeviceId::parse(DEVICE).unwrap();
        let caller = oxplow_domain::Caller {
            device: device.clone(),
            connection: oxplow_domain::ConnectionId::parse(ONE).unwrap(),
        };
        let (mut old, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let (mut new, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let now = oxplow_domain::Timestamp::now();
        svc.client_reports.report(
            &caller,
            oxplow_app::client_reports::TabReport {
                focused: true,
                visible: true,
                shown_thread: None,
                detail_json: None,
                runs: vec![],
            },
            now,
        );
        let gone = |svc: &std::sync::Arc<oxplow_app::Services>| {
            svc.client_devices.get(&device).is_none()
                && svc
                    .client_reports
                    .live_ids(oxplow_domain::Timestamp::now())
                    .is_empty()
        };
        old.close(None).await.unwrap();
        // The old socket's close has run once its read loop ends; give it the
        // time a close takes, then the window must still be there.
        for _ in 0..50 {
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            svc.client_devices.get(&device).is_some(),
            "the device stays connected"
        );
        assert_eq!(
            svc.client_reports.live_ids(oxplow_domain::Timestamp::now()),
            vec![ONE.to_string()],
            "the window's report stays"
        );
        new.close(None).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !gone(&svc) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the current socket's close releases the window");
    }

    #[tokio::test]
    async fn events_ws_streams_lsp_session_events() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        svc.lsp_sessions.emit_event_for_tests(
            oxplow_app::lsp_sessions::LspSessionEvent::SessionStatus {
                stream_id: "s-1".into(),
                language: "rust".into(),
                status: oxplow_app::lsp_sessions::LspSessionStatus::Crashed,
                message: Some("boom".into()),
            },
        );
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("ws frame within timeout")
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .to_string();
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["channel"], "lsp");
        assert_eq!(v["payload"]["kind"], "sessionStatus");
        assert_eq!(v["payload"]["status"], "crashed");
        assert_eq!(v["payload"]["streamId"], "s-1");
    }

    #[tokio::test]
    async fn events_ws_streams_terminal_events() {
        use futures::StreamExt as _;
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // Same subscribe-race handling as the oxplow/lsp events tests.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                svc.terminal_sessions.emit_event_for_tests(
                    oxplow_app::terminal_sessions::TerminalBridgeEvent {
                        session_id: "term-1".into(),
                        message: "{\"type\":\"data\",\"base64\":\"aGk=\"}".into(),
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
        assert_eq!(v["channel"], "terminal");
        assert_eq!(v["payload"]["sessionId"], "term-1");
        assert!(
            v["payload"]["message"].is_string(),
            "terminal frame carries the JSON-encoded protocol message"
        );
    }

    /// Terminal input written on the socket reaches the PTY in the order
    /// it was written, back to back with no answer between.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_terminal_frame_on_the_socket_reaches_the_pty_in_order() {
        use futures::SinkExt as _;
        let (svc, dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let uri = oxplow_domain::uri::build::terminal_uri(oxplow_domain::TerminalId::new(7));
        let capture = dir.path().join("typed.txt");
        let command = format!("cat > '{}'", capture.display());
        svc.terminal_sessions
            .attach_or_create(uri.clone(), 80, 24, |cols, rows| {
                oxplow_app::terminal_sessions::SpawnRequest {
                    command: "sh".into(),
                    args: vec!["-lc".into(), command],
                    cwd: dir.path().to_path_buf(),
                    env: oxplow_app::agent_path::base_pty_env(),
                    env_remove: vec![],
                    cols,
                    rows,
                }
            })
            .await
            .unwrap();
        let url = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let keys: Vec<String> = (0..50).map(|i| format!("W{i:02} ")).collect();
        for k in keys.iter().map(String::as_str).chain(["\r"]) {
            let bytes = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, k);
            let frame = serde_json::json!({
                "channel": "terminal",
                "uri": uri,
                "message": serde_json::json!({ "type": "input", "bytes": bytes }).to_string(),
            });
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                frame.to_string().into(),
            ))
            .await
            .unwrap();
        }
        let want: String = keys.concat();
        let got = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let text = std::fs::read_to_string(&capture).unwrap_or_default();
                if text.contains(want.trim_end()) {
                    return text;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the typed keys, in order");
        assert!(got.contains(want.trim_end()), "{got:?}");
    }

    #[tokio::test]
    async fn health_route_responds() {
        let (svc, _dir) = services().await;
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/health", daemon.bind_addr);
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert_eq!(body, "ok");
    }
}
