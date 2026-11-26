//! `/p/<project>/…`: forward to that project's daemon with the prefix
//! removed.
//!
//! `ipc/…`, `health`, `components/…`, `component-lib/…` go over HTTP and
//! `events` is a WebSocket tunnel. A client proves itself to the host with
//! its token (bearer on HTTP, `?token=` on the socket): the owner's or a
//! paired device's (`crate::access`). The host drops it and puts the daemon's
//! own token in its place, so a client never sees a daemon token. For a
//! device the host says who it is: `Oxplow-Device` and the socket's
//! `device`, `name`, `kind` come from its record, and only `Oxplow-Connection`
//! (the page load) passes as sent. The owner (the Mac app) names its own
//! device.
//!
//! The component routes carry no client token, as the daemon's carry none (a
//! sandboxed frame cannot send one). The `Host` check in front of every route
//! is their guard; the host tells the daemon the public base the frame is
//! reached by (`Oxplow-Public-Base`), which the daemon believes only with its
//! own token.

use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, RawQuery, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{SinkExt, StreamExt};
use oxplow_app::daemon_supervisor::DaemonEndpoint;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as UpstreamClose;
use tokio_tungstenite::tungstenite::Message as Upstream;

use crate::access::{Caller, Connected, TunnelLease};
use crate::api::bearer;
use crate::host::Reach;
use crate::ui;
use crate::web::{refuse, Web};

/// The header the host names its public base in; see `oxplow-daemon`'s
/// `components::PUBLIC_BASE_HEADER`.
const PUBLIC_BASE: &str = "oxplow-public-base";

/// Headers that belong to one connection, not to the message.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

/// Which daemon route a path under `/p/<project>/` names, and whether the
/// host token is needed for it.
fn daemon_route(rest: &str) -> Option<bool> {
    match rest.split('/').next() {
        Some("ipc") if rest.len() > 4 => Some(true),
        Some("health") if rest == "health" => Some(false),
        Some("components") if rest.len() > 11 => Some(false),
        Some("component-lib") if rest.len() > 14 => Some(false),
        _ => None,
    }
}

/// `/p/<project>/<rest>`: a daemon route, else a file of the UI.
pub(crate) async fn rest(
    State(web): State<Web>,
    Path((id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    // A dot or empty segment could climb out of the route it was checked as
    // (`components/../ipc/x` needs no token, and the URL parser makes `/ipc/x`
    // of it): never forwarded.
    if !plain_path(&rest) {
        return refuse(StatusCode::NOT_FOUND, "no such route");
    }
    let Some(needs_token) = daemon_route(&rest) else {
        return if req.method() == axum::http::Method::GET {
            ui::project_file(&web, &id, &rest).await
        } else {
            refuse(StatusCode::NOT_FOUND, "no such route")
        };
    };
    let device = if needs_token && !is_preflight(&req) {
        match caller(&web, req.headers(), None) {
            None => return refuse(StatusCode::UNAUTHORIZED, "missing or wrong token"),
            Some(Caller::Owner) => None,
            Some(Caller::Device(d)) => Some(d.id),
        }
    } else {
        None
    };
    let endpoint = match reach(&web, &id).await {
        Ok(endpoint) => endpoint,
        Err(refused) => return *refused,
    };
    forward(&web, &id, &rest, endpoint, req, device).await
}

/// A CORS preflight: a Mac window (`tauri://localhost`) asks before each
/// cross-origin call, and a browser never sends a token with it. It reads
/// nothing and changes nothing; the daemon's CORS layer answers it.
fn is_preflight(req: &Request) -> bool {
    req.method() == axum::http::Method::OPTIONS
        && req
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
}

/// Whether `rest` is plain names: no `.` or `..` segment, decoded or not,
/// no empty one but a trailing slash (a bundle's folder, `components/v/<v>/`),
/// and no backslash.
fn plain_path(rest: &str) -> bool {
    let decoded = crate::ui::percent_decode(rest);
    [rest, decoded.as_str()].iter().all(|path| {
        let inner = path.strip_suffix('/').unwrap_or(path);
        !path.contains('\\')
            && !path.chars().any(char::is_control)
            && inner
                .split('/')
                .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
    })
}

/// The parsed pairs of a query.
fn pairs(query: Option<&str>) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .into_owned()
        .collect()
}

/// Who a request is, by its bearer or (a socket) its `token` query.
fn caller(web: &Web, headers: &HeaderMap, query: Option<&[(String, String)]>) -> Option<Caller> {
    let from_query = query.and_then(|q| {
        q.iter()
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.as_str())
    });
    web.access
        .caller(web.listener, bearer(headers).or(from_query))
}

/// The daemon of `id`, waiting for one that is starting; the refusal to give
/// otherwise.
async fn reach(web: &Web, id: &str) -> Result<DaemonEndpoint, Box<Response>> {
    let deadline = tokio::time::Instant::now() + web.boot_wait;
    loop {
        match web.host.reach(id) {
            Reach::Ready(endpoint) => return Ok(endpoint),
            Reach::Starting if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Reach::Starting => {
                let mut r = down(StatusCode::SERVICE_UNAVAILABLE, id, "starting");
                r.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
                return Err(Box::new(r));
            }
            Reach::Unknown => {
                return Err(Box::new(refuse(StatusCode::NOT_FOUND, "no such project")))
            }
            Reach::Down(state) => {
                let (status, name) = match state {
                    crate::host::DaemonState::Crashed { .. } => {
                        (StatusCode::BAD_GATEWAY, "crashed")
                    }
                    crate::host::DaemonState::Stopping => (StatusCode::NOT_FOUND, "stopping"),
                    _ => (StatusCode::NOT_FOUND, "stopped"),
                };
                return Err(Box::new(down(status, id, name)));
            }
        }
    }
}

fn down(status: StatusCode, id: &str, state: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": "project not running",
            "project": id,
            "state": state,
        })),
    )
        .into_response()
}

/// The origin and prefix a browser reached this request by.
fn public_base(web: &Web, headers: &HeaderMap, id: &str) -> Option<String> {
    Some(format!(
        "{}/p/{id}",
        crate::ui::origin(web.scheme, headers)?
    ))
}

async fn forward(
    web: &Web,
    id: &str,
    rest: &str,
    endpoint: DaemonEndpoint,
    req: Request,
    device: Option<oxplow_domain::DeviceId>,
) -> Response {
    let (parts, body) = req.into_parts();
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("{}/{rest}{query}", endpoint.base_url);
    let Ok(url) = url.parse::<reqwest::Url>() else {
        return refuse(StatusCode::NOT_FOUND, "no such route");
    };
    // What reaches the daemon is the route that was checked, and no other.
    if daemon_route(url.path().trim_start_matches('/')) != daemon_route(rest) {
        tracing::warn!(project = %id, path = %rest, "a path that changes its route when parsed");
        return refuse(StatusCode::NOT_FOUND, "no such route");
    }
    let mut out = reqwest::Request::new(parts.method.clone(), url);
    for (name, value) in &parts.headers {
        let lower = name.as_str();
        if HOP_BY_HOP.contains(&lower) || lower == "authorization" || lower == PUBLIC_BASE {
            continue;
        }
        if device.is_some() && lower == "oxplow-device" {
            continue;
        }
        out.headers_mut().append(name.clone(), value.clone());
    }
    if let Some(device) = device
        .as_ref()
        .and_then(|d| HeaderValue::from_str(d.as_str()).ok())
    {
        out.headers_mut()
            .insert(HeaderName::from_static("oxplow-device"), device);
    }
    let token = HeaderValue::from_str(&format!("Bearer {}", endpoint.token)).expect("a header");
    out.headers_mut().insert(header::AUTHORIZATION, token);
    if let Some(base) =
        public_base(web, &parts.headers, id).and_then(|b| HeaderValue::from_str(&b).ok())
    {
        out.headers_mut()
            .insert(HeaderName::from_static(PUBLIC_BASE), base);
    }
    *out.body_mut() = Some(reqwest::Body::wrap_stream(body.into_data_stream()));

    let upstream = match web.client.execute(out).await {
        Ok(upstream) => upstream,
        Err(e) => {
            tracing::warn!(project = %id, error = %e, "the daemon did not answer");
            return refuse(StatusCode::BAD_GATEWAY, "the daemon did not answer");
        }
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = upstream.status();
    for (name, value) in upstream.headers() {
        if HOP_BY_HOP.contains(&name.as_str()) {
            continue;
        }
        let value = if name == header::LOCATION {
            prefixed(value, id)
        } else {
            value.clone()
        };
        response.headers_mut().append(name.clone(), value);
    }
    *response.body_mut() = Body::from_stream(upstream.bytes_stream());
    response
}

/// A redirect to a path on the daemon is a path under the project's prefix
/// for the browser.
fn prefixed(location: &HeaderValue, id: &str) -> HeaderValue {
    match location.to_str() {
        Ok(l) if l.starts_with('/') && !l.starts_with("//") => {
            HeaderValue::from_str(&format!("/p/{id}{l}")).unwrap_or_else(|_| location.clone())
        }
        _ => location.clone(),
    }
}

/// `/p/<project>/events`: the daemon's event socket, frame for frame.
pub(crate) async fn events(
    State(web): State<Web>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let given = pairs(query.as_deref());
    let Some(who) = caller(&web, &headers, Some(&given)) else {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong token");
    };
    let endpoint = match reach(&web, &id).await {
        Ok(endpoint) => endpoint,
        Err(refused) => return *refused,
    };
    let (sent, connected) = identity(&who, given);
    // Built apart: the serializer is not `Send`, and must not live across
    // the await below.
    let query = {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &sent {
            query.append_pair(k, v);
        }
        query.append_pair("token", &endpoint.token);
        query.finish()
    };
    let url = format!(
        "{}/events?{query}",
        endpoint.base_url.replacen("http", "ws", 1)
    );
    // Open the daemon's side first: a daemon that refuses is a 502 here, not
    // a socket that opens and closes.
    let upstream = match tokio_tungstenite::connect_async(&url).await {
        Ok((socket, _)) => socket,
        Err(e) => {
            tracing::warn!(project = %id, error = %e, "the daemon's event socket did not open");
            return refuse(StatusCode::BAD_GATEWAY, "the daemon did not answer");
        }
    };
    let lease = web.access.open_tunnel(connected);
    // Removed while this socket was being set up (the daemon's boot can take
    // a minute): its removal closed nothing, so ask again now it counts.
    if let Caller::Device(d) = &who {
        if !web
            .access
            .devices
            .list()
            .iter()
            .any(|known| known.id == d.id)
        {
            return refuse(StatusCode::UNAUTHORIZED, "missing or wrong token");
        }
    }
    ws.on_upgrade(move |client| tunnel(client, upstream, lease))
}

/// What the daemon is told about the socket's page load, and who it is for
/// the host's count. The client's token never passes. The owner (the Mac
/// app) says its own device; a device is what its record says, and only the
/// page load (`connection`) passes as it sent it. Rebuilt from the parsed
/// pairs, so no spelling of a key slips past.
fn identity(who: &Caller, given: Vec<(String, String)>) -> (Vec<(String, String)>, Connected) {
    let value = |key: &str| {
        given
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    match who {
        Caller::Owner => {
            let connected = Connected {
                device: value("device"),
                name: value("name"),
                kind: serde_json::from_value(serde_json::Value::String(value("kind"))).ok(),
            };
            let sent = given.into_iter().filter(|(k, _)| k != "token").collect();
            (sent, connected)
        }
        Caller::Device(d) => {
            let kind = serde_json::to_value(d.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let sent = vec![
                ("device".to_string(), d.id.to_string()),
                ("connection".to_string(), value("connection")),
                ("name".to_string(), d.name.clone()),
                ("kind".to_string(), kind),
            ];
            let connected = Connected {
                device: d.id.to_string(),
                name: d.name.clone(),
                kind: Some(d.kind),
            };
            (sent, connected)
        }
    }
}

type UpstreamSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Frames both ways, each direction in order. A send waits for the other
/// side to take the frame, so a slow reader slows the writer instead of the
/// host holding what it cannot deliver. It ends when either side does.
async fn tunnel(client: WebSocket, upstream: UpstreamSocket, lease: TunnelLease) {
    let (to_client, mut from_client) = client.split();
    let to_client = tokio::sync::Mutex::new(to_client);
    let (mut to_daemon, mut from_daemon) = upstream.split();
    let mut closed = lease.closed.clone();
    let up = async {
        while let Some(Ok(frame)) = from_client.next().await {
            let end = matches!(frame, Message::Close(_));
            if to_daemon.send(to_upstream(frame)).await.is_err() || end {
                break;
            }
        }
    };
    let down = async {
        while let Some(Ok(frame)) = from_daemon.next().await {
            let Some(frame) = to_client_frame(frame) else {
                continue;
            };
            let end = matches!(frame, Message::Close(_));
            if to_client.lock().await.send(frame).await.is_err() || end {
                break;
            }
        }
    };
    // The host closes it: the device was removed, or the host is ending.
    let ended = async {
        loop {
            if let Some(code) = *closed.borrow_and_update() {
                return code;
            }
            if closed.changed().await.is_err() {
                return std::future::pending::<u16>().await;
            }
        }
    };
    // Ends at once: no frame of the page passes once it is closed.
    let code = tokio::select! {
        _ = up => None,
        _ = down => None,
        code = ended => Some(code),
    };
    if let Some(code) = code {
        let reason = match code {
            crate::access::HOST_ENDING_CLOSE_CODE => "the host is ending",
            _ => "this device was removed",
        };
        let frame = Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }));
        // A page that reads nothing does not hold the host up.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            to_client.lock().await.send(frame),
        )
        .await;
    }
    drop(lease);
}

fn to_upstream(frame: Message) -> Upstream {
    match frame {
        Message::Text(t) => Upstream::Text(t.as_str().into()),
        Message::Binary(b) => Upstream::Binary(b.to_vec().into()),
        Message::Ping(b) => Upstream::Ping(b.to_vec().into()),
        Message::Pong(b) => Upstream::Pong(b.to_vec().into()),
        Message::Close(frame) => Upstream::Close(frame.map(|f| UpstreamClose {
            code: CloseCode::from(f.code),
            reason: f.reason.as_str().into(),
        })),
    }
}

/// `None` for a raw frame, which a reader never returns.
fn to_client_frame(frame: Upstream) -> Option<Message> {
    Some(match frame {
        Upstream::Text(t) => Message::Text(t.as_str().into()),
        Upstream::Binary(b) => Message::Binary(b.to_vec().into()),
        Upstream::Ping(b) => Message::Ping(b.to_vec().into()),
        Upstream::Pong(b) => Message::Pong(b.to_vec().into()),
        Upstream::Close(frame) => Message::Close(frame.map(|f| CloseFrame {
            code: u16::from(f.code),
            reason: f.reason.as_str().into(),
        })),
        Upstream::Frame(_) => return None,
    })
}
