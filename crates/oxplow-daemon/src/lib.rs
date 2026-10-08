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
//! reach it through `ssh -L <localPort>:127.0.0.1:<port>`.
//!
//! **Only the person's UI may call it** (tsk345). `/ipc` and `/events`
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
    /// The UI token every `/ipc` call (`Authorization: Bearer`) and the
    /// `/events` socket (`?token=`) must carry.
    pub token: String,
    pub ctx: oxplow_rpc::RpcContext,
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

/// `GET /events` — WebSocket multiplexing the backend event channels
/// (`oxplow:event`, `lsp:event`, `terminal:event`, `acp:event`). Each
/// frame is `{"channel":"oxplow"|"lsp"|"terminal"|"acp","payload":<event>}`
/// with the event's serialized shape, so the renderer's handlers are
/// transport-agnostic.
///
/// The channels are subscribed here, before the upgrade is answered
/// (tsk995): the upgrade's callback runs only after the client has its
/// `101`, so a client that writes once its socket opens could otherwise
/// cause an event no forwarder was there to see.
async fn events_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    let forwarders = subscribe(&state);
    ws.on_upgrade(move |socket| events_stream(socket, forwarders))
}

/// One channel's subscription, waiting for the socket's queue to forward
/// into.
type Forwarder =
    Box<dyn FnOnce(tokio::sync::mpsc::Sender<String>) -> tokio::task::JoinHandle<()> + Send>;

/// A subscription to `rx`, each event framed by `frame`. A lagged
/// subscriber just drops frames — the renderer's coarse "bucket changed,
/// refetch" model recovers on the next event (and refetches everything on
/// reconnect anyway).
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
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
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

/// Subscribe every channel the socket carries. Frame keys come from the
/// shared channel registry so the daemon, the Tauri shell, and the
/// renderer's demux table can't drift.
fn subscribe(state: &DaemonState) -> [Forwarder; 4] {
    let [oxplow_key, lsp_key, terminal_key, acp_key] = ws_frame_keys();
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
        forward(state.ctx.acp.subscribe(), move |e| frame_json(acp_key, e)),
    ]
}

/// The `/events` frame keys in [oxplow, lsp, terminal, acp] order,
/// resolved from `oxplow_app::event_channels::FRAMES` by the channel
/// each key demuxes onto.
fn ws_frame_keys() -> [&'static str; 4] {
    use oxplow_app::event_channels as ch;
    let key_for = |channel: &str| -> &'static str {
        ch::FRAMES
            .iter()
            .find(|(_, c)| *c == channel)
            .map(|(k, _)| *k)
            .unwrap_or_else(|| unreachable!("channel {channel} missing from FRAMES"))
    };
    [
        key_for(ch::OXPLOW),
        key_for(ch::LSP),
        key_for(ch::TERMINAL),
        key_for(ch::ACP),
    ]
}

/// Start each channel's forwarder into one queue, then pump the socket
/// from it.
async fn events_stream(socket: WebSocket, subscriptions: [Forwarder; 4]) {
    let (mut sink, mut inbound) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let forwarders = subscriptions.map(|start| start(tx.clone()));
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
    use oxplow_domain::stores::StreamStore as _;
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
        let (svc, _dir) = services();
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
        let (svc, dir) = services();
        let ext = dir.path().join("oxplow/extensions/x");
        std::fs::create_dir_all(ext.join("components/c/assets")).unwrap();
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: c }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/c/index.html"), "<!doctype html>hi").unwrap();
        std::fs::write(ext.join("components/c/assets/app.js"), "1").unwrap();
        let version = load(&svc, dir.path(), "x", "c");
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
                components::bundle_csp(
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
        // tsk984: what's served is the snapshot — the disk changing after
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
    fn load(svc: &Arc<Services>, root: &std::path::Path, ext: &str, component: &str) -> String {
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
        let (svc, _dir) = services();
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
        // tsk961: the kit's stylesheet, as CSS.
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
        let (svc, dir) = services();
        let ext = dir.path().join("oxplow/extensions/x");
        std::fs::create_dir_all(ext.join("components/c")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: c }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/c/index.html"), "hi").unwrap();
        let version = load(&svc, dir.path(), "x", "c");
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
                components::bundle_csp(
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

    /// A stream's lens loads its component from the stream's worktree: a
    /// version of its own, served from what that worktree held.
    #[tokio::test]
    async fn a_streams_bundle_is_loaded_from_its_worktree() {
        let (svc, dir) = services();
        let wt = tempfile::tempdir().unwrap();
        for (root, js) in [(dir.path(), "primary"), (wt.path(), "stream")] {
            let ext = root.join("oxplow/extensions/x");
            std::fs::create_dir_all(ext.join("components/c")).unwrap();
            std::fs::write(
                ext.join("extension.yaml"),
                "manifest: 2\nname: x\nintent:\n  purpose: p\ncustom_components:\n  - { id: c }\n",
            )
            .unwrap();
            std::fs::write(ext.join("components/c/index.html"), "hi").unwrap();
            std::fs::write(ext.join("components/c/app.js"), js).unwrap();
        }
        let ts = oxplow_domain::Timestamp::from_unix_ms(1_700_000_000_000);
        svc.stream_store
            .upsert(&oxplow_domain::Stream {
                id: oxplow_domain::StreamId::new(2),
                kind: oxplow_domain::StreamKind::Worktree,
                title: "w".into(),
                branch: "w".into(),
                branch_ref: "refs/heads/w".into(),
                branch_source: "main".into(),
                worktree_path: wt.path().to_string_lossy().into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                host: oxplow_domain::HostId::LOCAL,
                custom_prompt: None,
                created_at: ts,
                updated_at: ts,
                archived_at: None,
            })
            .await
            .unwrap();
        let primary = load(&svc, dir.path(), "x", "c");
        let stream = load(&svc, wt.path(), "x", "c");
        assert_ne!(primary, stream);
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let base = format!("http://{}", daemon.bind_addr);
        let text = |version: String| {
            let url = format!("{base}/components/v/{version}/app.js");
            async move { reqwest::get(url).await.unwrap().text().await.unwrap() }
        };
        assert_eq!(text(stream).await, "stream");
        assert_eq!(text(primary).await, "primary");
    }

    #[tokio::test]
    async fn ipc_ping_returns_ok_envelope() {
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/get_effort", daemon.bind_addr);
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

    /// tsk995: the socket listens before it opens — an event emitted the
    /// moment a client sees the upgrade reaches it, so a client can open
    /// the socket, then write, and never miss what its write caused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_event_emitted_as_the_socket_opens_arrives() {
        use futures::StreamExt as _;
        let (svc, _dir) = services();
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

    #[tokio::test]
    async fn events_ws_streams_lsp_session_events() {
        use futures::StreamExt as _;
        let (svc, _dir) = services();
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
        let (svc, _dir) = services();
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

    #[tokio::test]
    async fn events_ws_streams_acp_events() {
        use futures::StreamExt as _;
        use oxplow_app::acp::session::{AcpEvent, AcpEventBody, AcpStatus};
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc.clone()))
            .await
            .unwrap();
        let url = format!("ws://{}/events?token={UI_TOKEN}", daemon.bind_addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // Same subscribe-race handling as the other events tests.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                svc.acp.emit_event_for_tests(AcpEvent {
                    agent_session_id: "ses4".into(),
                    thread_id: "thr3".into(),
                    generation: 1,
                    body: AcpEventBody::Status {
                        status: AcpStatus::Running,
                    },
                });
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
        assert_eq!(v["channel"], "acp");
        assert_eq!(v["payload"]["agentSessionId"], "ses4");
        assert_eq!(v["payload"]["threadId"], "thr3");
        assert_eq!(v["payload"]["type"], "status");
        assert_eq!(v["payload"]["status"], "running");
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
