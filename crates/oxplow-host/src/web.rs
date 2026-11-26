//! The host's whole HTTP surface: its own API (`/host/…`), the project
//! routes (`/p/<project>/…`, [`crate::proxy`] and [`crate::ui`]) and the
//! built UI at `/`.
//!
//! **Every request first passes the `Host` check.** A page on another name
//! whose DNS now points here (rebinding) would otherwise read the project
//! page, which carries the host token. Only the loopback names and the names
//! given with `--allow-host` are served.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use oxplow_app::component_http::loopback_host;

use crate::host::Host;
use crate::{api, proxy, ui};

/// What `serve` is told.
pub struct ServeConfig {
    pub host: Arc<Host>,
    /// The host token, until pairing gives each device its own.
    pub token: String,
    /// The built UI (`apps/desktop/dist`), read at run time. `None`: no UI.
    pub ui_dir: Option<PathBuf>,
    /// Names, besides loopback, a request may reach the host by.
    pub allow_hosts: Vec<String>,
    /// How long a request for a `starting` project waits for its daemon.
    pub boot_wait: Duration,
    /// Told when a client asks the host to stop (`POST /host/shutdown`).
    pub stop: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
pub(crate) struct Web {
    pub host: Arc<Host>,
    pub token: Arc<str>,
    pub ui_dir: Option<Arc<PathBuf>>,
    pub allow_hosts: Arc<Vec<String>>,
    pub boot_wait: Duration,
    pub client: reqwest::Client,
}

pub fn app(cfg: ServeConfig) -> Router {
    let web = Web {
        host: cfg.host.clone(),
        token: cfg.token.clone().into(),
        ui_dir: cfg.ui_dir.map(Arc::new),
        allow_hosts: Arc::new(
            cfg.allow_hosts
                .iter()
                .map(|h| h.to_ascii_lowercase())
                .collect(),
        ),
        boot_wait: cfg.boot_wait,
        // No redirect is followed and no body is read whole: the browser
        // gets the daemon's answer as it is.
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("an HTTP client"),
    };
    let project = Router::new()
        .route("/", get(ui::launcher))
        .route("/p/{id}", get(ui::project_redirect))
        .route("/p/{id}/", get(ui::project_page))
        .route("/p/{id}/events", get(proxy::events))
        .route("/p/{id}/{*rest}", any(proxy::rest))
        .fallback(ui::root_file)
        .with_state(web.clone());
    api::router(cfg.host, cfg.token, cfg.stop)
        .merge(project)
        .layer(axum::middleware::from_fn_with_state(web, check_host))
}

/// The lower-case name in a `Host` value, without its port (an IPv6 literal
/// keeps its brackets); `None` for a port that is not digits.
fn host_name(host: &str) -> Option<String> {
    let (name, port) = match host.find(']') {
        Some(end) if host.starts_with('[') => (&host[..=end], host[end + 1..].strip_prefix(':')),
        _ => match host.rsplit_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        },
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok.then(|| name.to_ascii_lowercase())
}

/// Whether a request that says `host` may be served: this machine's
/// loopback (the same names the daemon serves component frames to) or a name
/// the person allowed.
pub(crate) fn host_allowed(host: &str, allow: &[String]) -> bool {
    loopback_host(host).is_some() || host_name(host).is_some_and(|name| allow.contains(&name))
}

async fn check_host(State(web): State<Web>, req: Request, next: Next) -> Response {
    let allowed = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| host_allowed(h, &web.allow_hosts));
    if allowed {
        next.run(req).await
    } else {
        refuse(StatusCode::FORBIDDEN, "host not allowed")
    }
}

/// A short JSON reason.
pub(crate) fn refuse(status: StatusCode, reason: &str) -> Response {
    (status, Json(serde_json::json!({ "error": reason }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::Fixture;
    use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
    use axum::extract::Path;
    use axum::http::HeaderMap;
    use futures::{SinkExt, StreamExt};
    use std::sync::Mutex;
    use tokio_tungstenite::tungstenite::Message as Client;

    const HOST_TOKEN: &str = "host-secret";

    /// What the fake daemon saw, for the test to compare with what it sent.
    #[derive(Default)]
    struct Seen {
        paths: Vec<String>,
        tokens: Vec<String>,
    }

    type Log = Arc<Mutex<Seen>>;

    /// A daemon's routes, reduced to what the host forwards: one echo of
    /// the headers that matter, a redirect, a bundle, a library file and
    /// the event socket.
    fn fake_daemon(log: Log) -> Router {
        let log_ipc = log.clone();
        let log_ws = log.clone();
        let log_lib = log.clone();
        Router::new()
            .route(
                "/ipc/{name}",
                axum::routing::post(
                    move |Path(name): Path<String>,
                          uri: axum::http::Uri,
                          h: HeaderMap,
                          body: String| {
                        let log = log_ipc.clone();
                        async move {
                            let get = |n: &str| {
                                h.get(n).and_then(|v| v.to_str().ok()).map(str::to_string)
                            };
                            let auth = get("authorization");
                            {
                                let mut seen = log.lock().unwrap();
                                seen.paths.push(format!("/ipc/{name}"));
                                seen.tokens.extend(auth.clone());
                            }
                            (
                                [("x-from-daemon", "yes")],
                                Json(serde_json::json!({
                                    "name": name,
                                    "query": uri.query(),
                                    "auth": auth,
                                    "device": get("oxplow-device"),
                                    "connection": get("oxplow-connection"),
                                    "publicBase": get("oxplow-public-base"),
                                    "body": body,
                                })),
                            )
                        }
                    },
                ),
            )
            .route("/health", get(|| async { "ok" }))
            .route(
                "/components/v/abc",
                get(|| async {
                    (
                        StatusCode::PERMANENT_REDIRECT,
                        [(header::LOCATION, "/components/v/abc/")],
                    )
                }),
            )
            .route(
                "/components/v/abc/",
                get(move |h: HeaderMap| async move {
                    let base = h
                        .get("oxplow-public-base")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("none")
                        .to_string();
                    let has_auth = h.contains_key("authorization");
                    ([("x-public-base", base)], format!("bundle auth={has_auth}"))
                }),
            )
            .route(
                "/component-lib/{file}",
                get(move |Path(file): Path<String>| {
                    let log = log_lib.clone();
                    async move {
                        log.lock()
                            .unwrap()
                            .paths
                            .push(format!("/component-lib/{file}"));
                        "lib"
                    }
                }),
            )
            .route(
                "/events",
                get(move |ws: WebSocketUpgrade, uri: axum::http::Uri| {
                    let log = log_ws.clone();
                    async move {
                        let query = uri.query().unwrap_or("").to_string();
                        ws.on_upgrade(move |socket| events(socket, query, log))
                    }
                }),
            )
    }

    /// The event socket: says its query, pings, beats, echoes text and
    /// closes with a code of its own on `bye`.
    async fn events(mut socket: WebSocket, query: String, log: Log) {
        for kv in query.split('&') {
            if let Some(t) = kv.strip_prefix("token=") {
                log.lock().unwrap().tokens.push(t.to_string());
            }
        }
        let _ = socket
            .send(Message::Text(format!("query:{query}").into()))
            .await;
        let _ = socket.send(Message::Ping(Vec::new().into())).await;
        let _ = socket
            .send(Message::Text(r#"{"channel":"beat"}"#.into()))
            .await;
        while let Some(Ok(msg)) = socket.recv().await {
            match msg {
                Message::Text(t) if t.as_str() == "bye" => {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame {
                            code: 4001,
                            reason: "replaced".into(),
                        })))
                        .await;
                    return;
                }
                Message::Text(t) => {
                    let _ = socket.send(Message::Text(format!("echo:{t}").into())).await;
                }
                Message::Binary(b) => {
                    let _ = socket.send(Message::Binary(b)).await;
                }
                _ => {}
            }
        }
    }

    /// A host over a fake daemon, served on loopback.
    struct Rig {
        fx: Fixture,
        url: String,
        id: String,
        log: Log,
        ui: tempfile::TempDir,
    }

    async fn serve(router: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await });
        addr
    }

    impl Rig {
        /// `lead` runs in the daemon's script before it reports its address.
        async fn new(lead: &str, boot_wait: Duration, allow: &[&str]) -> Self {
            let log = Log::default();
            let daemon = serve(fake_daemon(log.clone())).await;
            let script =
                format!("{lead} echo 'oxplow-daemon listening on http://{daemon}'; sleep 30");
            let fx = Fixture::new(&script);
            let id = fx.project("alpha");
            let ui = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(ui.path().join("assets")).unwrap();
            std::fs::write(
                ui.path().join("index.html"),
                "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"frame-src blob: http://127.0.0.1:*\"><title>t</title></head><body>app</body></html>",
            )
            .unwrap();
            std::fs::write(ui.path().join("assets/app.js"), "console.log(1)").unwrap();
            std::fs::write(ui.path().parent().unwrap().join("secret.txt"), "no").ok();
            let app = app(ServeConfig {
                host: fx.host.clone(),
                token: HOST_TOKEN.into(),
                ui_dir: Some(ui.path().to_path_buf()),
                allow_hosts: allow.iter().map(|s| s.to_string()).collect(),
                boot_wait,
                stop: Default::default(),
            });
            let addr = serve(app).await;
            Self {
                fx,
                url: format!("http://{addr}"),
                id,
                log,
                ui,
            }
        }

        async fn running() -> Self {
            let rig = Self::new("", Duration::from_secs(10), &[]).await;
            rig.hold().await;
            rig
        }

        async fn hold(&self) {
            let id = self.id.clone();
            let host = self.fx.host.clone();
            tokio::task::spawn_blocking(move || host.add_hold("dev", &id).unwrap())
                .await
                .unwrap();
        }

        async fn wait_running(&self) {
            for _ in 0..400 {
                if matches!(self.fx.host.reach(&self.id), crate::host::Reach::Ready(_)) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("the daemon never ran");
        }

        fn daemon_token(&self) -> String {
            match self.fx.host.reach(&self.id) {
                crate::host::Reach::Ready(ep) => ep.token,
                other => panic!("not running: {other:?}"),
            }
        }

        fn p(&self, rest: &str) -> String {
            format!("{}/p/{}/{rest}", self.url, self.id)
        }

        fn ws(&self, query: &str) -> String {
            format!(
                "ws://{}/p/{}/events?{query}",
                self.url.trim_start_matches("http://"),
                self.id
            )
        }
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn an_ipc_call_reaches_the_daemon_without_the_prefix_and_with_its_own_token() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        let reply = http()
            .post(rig.p("ipc/list_streams?x=1"))
            .bearer_auth(HOST_TOKEN)
            .header("oxplow-device", "dev-1")
            .header("oxplow-connection", "con-1")
            // A client cannot say where the host is reached from.
            .header("oxplow-public-base", "http://evil.example/p/x")
            .body("{\"a\":1}")
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 200);
        assert_eq!(reply.headers()["x-from-daemon"], "yes");
        let seen: serde_json::Value = reply.json().await.unwrap();
        assert_eq!(seen["name"], "list_streams");
        assert_eq!(seen["query"], "x=1");
        assert_eq!(seen["body"], "{\"a\":1}");
        assert_eq!(seen["device"], "dev-1");
        assert_eq!(seen["connection"], "con-1");
        let token = rig.daemon_token();
        assert_eq!(seen["auth"], format!("Bearer {token}"));
        assert_ne!(token, HOST_TOKEN);
        assert_eq!(
            seen["publicBase"],
            format!(
                "http://{}/p/{}",
                rig.url.trim_start_matches("http://"),
                rig.id
            )
        );
    }

    #[tokio::test]
    async fn an_ipc_call_without_the_host_token_never_reaches_the_daemon() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        for req in [
            http().post(rig.p("ipc/x")),
            http().post(rig.p("ipc/x")).bearer_auth("nope"),
            // The daemon's own token is not the host's.
            http().post(rig.p("ipc/x")).bearer_auth(rig.daemon_token()),
        ] {
            assert_eq!(req.send().await.unwrap().status(), 401);
        }
        assert!(rig.log.lock().unwrap().paths.is_empty());
    }

    #[tokio::test]
    async fn health_and_components_need_no_client_token_but_the_daemon_gets_its_own() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        let health = http().get(rig.p("health")).send().await.unwrap();
        assert_eq!(health.status(), 200);
        assert_eq!(health.text().await.unwrap(), "ok");
        let lib = http()
            .get(rig.p("component-lib/a.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(lib.text().await.unwrap(), "lib");

        let bundle = http().get(rig.p("components/v/abc/")).send().await.unwrap();
        let origin = rig.url.trim_start_matches("http://");
        assert_eq!(
            bundle.headers()["x-public-base"].to_str().unwrap(),
            format!("http://{origin}/p/{}", rig.id)
        );
        assert_eq!(bundle.text().await.unwrap(), "bundle auth=true");
    }

    #[tokio::test]
    async fn a_redirect_from_the_daemon_keeps_the_project_prefix() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        let resp = http().get(rig.p("components/v/abc")).send().await.unwrap();
        assert_eq!(resp.status(), 308);
        assert_eq!(
            resp.headers()["location"].to_str().unwrap(),
            format!("/p/{}/components/v/abc/", rig.id)
        );
    }

    async fn connect(
        url: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    async fn next_msg(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Client {
        tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a frame in time")
            .expect("the socket stays open")
            .unwrap()
    }

    #[tokio::test]
    async fn the_event_socket_is_a_tunnel_with_the_daemons_token_and_the_clients_identity() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        let mut ws = connect(&rig.ws(&format!(
            "token={HOST_TOKEN}&device=dev-1&connection=con-1&name=Mac&kind=desktop"
        )))
        .await;
        let Client::Text(first) = next_msg(&mut ws).await else {
            panic!("the query frame first");
        };
        let token = rig.daemon_token();
        assert!(first.contains(&format!("token={token}")), "{first}");
        assert!(!first.contains(HOST_TOKEN), "{first}");
        for kept in [
            "device=dev-1",
            "connection=con-1",
            "name=Mac",
            "kind=desktop",
        ] {
            assert!(first.contains(kept), "{first}");
        }
        // The daemon's ping and its beat pass through, in order.
        assert!(matches!(next_msg(&mut ws).await, Client::Ping(_)));
        let Client::Text(beat) = next_msg(&mut ws).await else {
            panic!("the beat");
        };
        assert_eq!(beat.as_str(), r#"{"channel":"beat"}"#);
        // Frames the client sends come back in the order sent.
        for n in 0..200 {
            ws.send(Client::Text(format!("n{n}").into())).await.unwrap();
        }
        for n in 0..200 {
            let Client::Text(t) = next_msg(&mut ws).await else {
                panic!("an echo");
            };
            assert_eq!(t.as_str(), format!("echo:n{n}"));
        }
        ws.send(Client::Binary(vec![1, 2, 3].into())).await.unwrap();
        assert_eq!(
            next_msg(&mut ws).await,
            Client::Binary(vec![1, 2, 3].into())
        );
        // The daemon's own close code reaches the client.
        ws.send(Client::Text("bye".into())).await.unwrap();
        let Client::Close(Some(frame)) = next_msg(&mut ws).await else {
            panic!("a close");
        };
        assert_eq!(u16::from(frame.code), 4001);
        assert_eq!(frame.reason.as_str(), "replaced");
    }

    #[tokio::test]
    async fn the_event_socket_needs_the_host_token() {
        let rig = Rig::running().await;
        rig.wait_running().await;
        for query in ["", "token=nope", "device=d"] {
            let refused = tokio_tungstenite::connect_async(rig.ws(query)).await;
            assert!(refused.is_err(), "{query}");
        }
        assert!(rig.log.lock().unwrap().tokens.is_empty());
    }

    #[tokio::test]
    async fn a_project_that_is_not_running_is_a_404_with_the_reason() {
        let rig = Rig::new("", Duration::from_secs(1), &[]).await;
        // Not held.
        let resp = http()
            .post(rig.p("ipc/x"))
            .bearer_auth(HOST_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["state"], "stopped");
        assert!(body["error"].as_str().unwrap().contains("not running"));
        // A project the registry has not: told apart.
        let unknown = http()
            .post(format!("{}/p/prj-00000000/ipc/x", rig.url))
            .bearer_auth(HOST_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 404);
        let body: serde_json::Value = unknown.json().await.unwrap();
        assert_eq!(body["error"], "no such project");
        // Held, then let go: stopped again.
        rig.hold().await;
        rig.wait_running().await;
        let (host, id) = (rig.fx.host.clone(), rig.id.clone());
        tokio::task::spawn_blocking(move || host.release_hold("dev", &id).unwrap())
            .await
            .unwrap();
        rig.fx
            .wait_for(&rig.id, |s| *s == crate::host::DaemonState::Stopped);
        let after = http().get(rig.p("health")).send().await.unwrap();
        assert_eq!(after.status(), 404);
        // The socket is refused the same way, before any upgrade.
        assert!(
            tokio_tungstenite::connect_async(rig.ws(&format!("token={HOST_TOKEN}")))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_daemon_that_crashed_is_a_502() {
        let rig = Rig::new("exit 1;", Duration::from_secs(1), &[]).await;
        rig.hold().await;
        rig.fx.wait_for(&rig.id, |s| {
            matches!(s, crate::host::DaemonState::Crashed { .. })
        });
        let resp = http().get(rig.p("health")).send().await.unwrap();
        assert_eq!(resp.status(), 502);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["state"], "crashed");
    }

    #[tokio::test]
    async fn a_starting_project_waits_for_its_daemon() {
        let rig = Rig::new("sleep 1;", Duration::from_secs(10), &[]).await;
        rig.hold().await;
        // The daemon is not up yet; the request waits and then goes through.
        let resp = http().get(rig.p("health")).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn a_daemon_that_boots_too_slowly_is_a_503_not_a_hang() {
        let rig = Rig::new("sleep 1;", Duration::from_millis(200), &[]).await;
        rig.hold().await;
        let resp = http().get(rig.p("health")).send().await.unwrap();
        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["state"], "starting");
        // Let it finish booting, so the test leaves no daemon behind.
        rig.wait_running().await;
    }

    #[tokio::test]
    async fn only_an_allowed_host_name_is_served() {
        let rig = Rig::new("", Duration::from_secs(10), &["Tablet.LAN"]).await;
        rig.hold().await;
        rig.wait_running().await;
        let port = rig.url.rsplit(':').next().unwrap().to_string();
        let url = rig.url.clone();
        let get = |path: String, host: String| {
            let url = url.clone();
            async move {
                http()
                    .get(format!("{url}{path}"))
                    .header("host", host)
                    .send()
                    .await
                    .unwrap()
            }
        };
        for host in [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("tablet.lan:{port}"),
            "tablet.lan".to_string(),
        ] {
            for path in [
                "/",
                "/host/status",
                &format!("/p/{}/", rig.id),
                &format!("/p/{}/health", rig.id),
            ] {
                let resp = get(path.to_string(), host.clone()).await;
                assert_ne!(resp.status(), 403, "{host} {path}");
            }
        }
        for host in [
            format!("evil.example:{port}"),
            "evil.example".to_string(),
            format!("tablet.lan.evil.example:{port}"),
            format!("127.0.0.1.evil.example:{port}"),
        ] {
            for path in [
                "/",
                "/host/status",
                &format!("/p/{}/", rig.id),
                &format!("/p/{}/health", rig.id),
                &format!("/p/{}/components/v/abc/", rig.id),
            ] {
                let resp = get(path.to_string(), host.clone()).await;
                assert_eq!(resp.status(), 403, "{host:?} {path}");
            }
        }
        // And the socket.
        let mut req =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                rig.ws(&format!("token={HOST_TOKEN}")),
            )
            .unwrap();
        req.headers_mut()
            .insert("host", "evil.example".parse().unwrap());
        assert!(tokio_tungstenite::connect_async(req).await.is_err());
    }

    #[test]
    fn host_names_are_compared_without_the_port_and_case() {
        let allow = vec!["tablet.lan".to_string(), "[fd00::1]".to_string()];
        for ok in [
            "tablet.lan",
            "Tablet.Lan:7400",
            "[fd00::1]:7400",
            "127.0.0.1:1",
        ] {
            assert!(host_allowed(ok, &allow), "{ok}");
        }
        for bad in [
            "",
            "tablet.lan:",
            "tablet.lan:x",
            "tablet.lan.evil",
            "evil",
            "[fd00::2]:1",
        ] {
            assert!(!host_allowed(bad, &allow), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_project_page_carries_its_window_context() {
        let rig = Rig::new("", Duration::from_secs(10), &[]).await;
        let resp = http().get(rig.p("")).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "text/html; charset=utf-8");
        let html = resp.text().await.unwrap();
        let origin = rig.url.trim_start_matches("http://");
        let script = html
            .split("<script>window.__OXPLOW__ = Object.freeze(")
            .nth(1)
            .expect("the context is injected")
            .split(");</script>")
            .next()
            .unwrap();
        let ctx: serde_json::Value = serde_json::from_str(script).unwrap();
        assert_eq!(ctx["kind"], "project");
        assert_eq!(ctx["project"], rig.id);
        assert_eq!(ctx["base"], format!("http://{origin}/p/{}", rig.id));
        assert_eq!(ctx["token"], HOST_TOKEN);
        // First in the head, before any script of the app.
        assert!(html.contains("<head><script>window.__OXPLOW__"), "{html}");
        // Frames may come from the page's own origin.
        assert!(
            html.contains(&format!("frame-src blob: http://{origin} ")),
            "{html}"
        );
        assert!(html.contains("<body>app</body>"));
    }

    #[tokio::test]
    async fn the_page_and_its_files_load_under_any_project_prefix_and_at_the_root() {
        let rig = Rig::new("", Duration::from_secs(10), &[]).await;
        for path in [
            format!("/p/{}/assets/app.js", rig.id),
            "/assets/app.js".to_string(),
        ] {
            let resp = http()
                .get(format!("{}{path}", rig.url))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200, "{path}");
            assert_eq!(
                resp.headers()["content-type"],
                "text/javascript; charset=utf-8"
            );
            assert_eq!(resp.text().await.unwrap(), "console.log(1)");
        }
        // The folder form redirects, so relative URLs resolve inside it.
        let bare = http()
            .get(format!("{}/p/{}", rig.url, rig.id))
            .send()
            .await
            .unwrap();
        assert_eq!(bare.status(), 308);
        assert_eq!(
            bare.headers()["location"].to_str().unwrap(),
            format!("/p/{}/", rig.id)
        );
        // A project the registry lacks has no page; a missing file is a 404.
        for path in [
            "/p/prj-00000000/".to_string(),
            "/p/prj-00000000/assets/app.js".to_string(),
            format!("/p/{}/assets/nope.js", rig.id),
            "/assets/nope.js".to_string(),
        ] {
            let resp = http()
                .get(format!("{}{path}", rig.url))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{path}");
        }
        drop(rig.ui);
    }

    #[tokio::test]
    async fn no_path_leaves_the_ui_folder() {
        let rig = Rig::new("", Duration::from_secs(10), &[]).await;
        for path in [
            "/%2e%2e/secret.txt",
            "/assets/%2e%2e/%2e%2e/secret.txt",
            "/..%2fsecret.txt",
            &format!("/p/{}/..%2fsecret.txt", rig.id),
            "//etc/passwd",
            "/assets//app.js",
        ] {
            let resp = http()
                .get(format!("{}{path}", rig.url))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{path}");
        }
    }

    #[tokio::test]
    async fn the_root_lists_the_held_projects_and_escapes_their_names() {
        let rig = Rig::new("", Duration::from_secs(10), &[]).await;
        let empty = http()
            .get(&rig.url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(empty.contains("No project is held"), "{empty}");
        assert!(!empty.contains(&rig.id));
        rig.hold().await;
        let page = http().get(&rig.url).send().await.unwrap();
        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
        let html = page.text().await.unwrap();
        assert!(html.contains(&format!("href=\"/p/{}/\"", rig.id)), "{html}");
        assert!(html.contains("alpha"), "{html}");
        // Only names: paths of the person's machine stay off a page anyone
        // who reaches the host may load.
        assert!(
            !html.contains(rig.fx.dir.path().to_str().unwrap()),
            "{html}"
        );
        let hostile = rig.fx.project("<b>x&\"'");
        let host = rig.fx.host.clone();
        tokio::task::spawn_blocking(move || host.add_hold("dev", &hostile).unwrap())
            .await
            .unwrap();
        let html = http()
            .get(&rig.url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains("&lt;b&gt;x&amp;&quot;&#39;"), "{html}");
        assert!(!html.contains("<b>x"), "{html}");
    }

    #[tokio::test]
    async fn with_no_ui_folder_the_pages_say_so() {
        let fx = Fixture::new(crate::host::tests::RUNS);
        let id = fx.project("alpha");
        let addr = serve(app(ServeConfig {
            host: fx.host.clone(),
            token: HOST_TOKEN.into(),
            ui_dir: None,
            allow_hosts: vec![],
            boot_wait: Duration::from_secs(1),
            stop: Default::default(),
        }))
        .await;
        for path in [format!("/p/{id}/"), "/assets/app.js".to_string()] {
            let resp = http()
                .get(format!("http://{addr}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{path}");
            let body: serde_json::Value = resp.json().await.unwrap();
            assert!(body["error"].as_str().unwrap().contains("--ui"), "{body}");
        }
        // The list needs no UI folder.
        let root = http().get(format!("http://{addr}/")).send().await.unwrap();
        assert_eq!(root.status(), 200);
    }
}
