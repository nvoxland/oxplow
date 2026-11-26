//! `/p/<project>/…`: forward to that project's daemon with the prefix
//! removed.
//!
//! `ipc/…`, `health`, `components/…`, `component-lib/…` go over HTTP and
//! `events` is a WebSocket tunnel. A client proves itself to the host with
//! the host token (bearer on HTTP, `?token=` on the socket, until pairing);
//! the host drops it and puts the daemon's own token in its place, so a
//! client never sees a daemon token. `Oxplow-Device` and `Oxplow-Connection`
//! (and the socket's device query) pass as the client sent them.
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

use crate::api::{bearer, same_secret};
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
    let Some(needs_token) = daemon_route(&rest) else {
        return if req.method() == axum::http::Method::GET {
            ui::project_file(&web, &id, &rest).await
        } else {
            refuse(StatusCode::NOT_FOUND, "no such route")
        };
    };
    if needs_token && !host_token_ok(&web, req.headers(), None) {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong host token");
    }
    let endpoint = match reach(&web, &id).await {
        Ok(endpoint) => endpoint,
        Err(refused) => return *refused,
    };
    forward(&web, &id, &rest, endpoint, req).await
}

fn host_token_ok(web: &Web, headers: &HeaderMap, query: Option<&str>) -> bool {
    let from_query = query.and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
    bearer(headers)
        .or(from_query)
        .is_some_and(|t| same_secret(t, &web.token))
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
fn public_base(headers: &HeaderMap, id: &str) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    Some(format!("http://{host}/p/{id}"))
}

async fn forward(
    web: &Web,
    id: &str,
    rest: &str,
    endpoint: DaemonEndpoint,
    req: Request,
) -> Response {
    let (parts, body) = req.into_parts();
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("{}/{rest}{query}", endpoint.base_url);
    let mut out = reqwest::Request::new(parts.method.clone(), url.parse().expect("a URL"));
    for (name, value) in &parts.headers {
        let lower = name.as_str();
        if HOP_BY_HOP.contains(&lower) || lower == "authorization" || lower == PUBLIC_BASE {
            continue;
        }
        out.headers_mut().append(name.clone(), value.clone());
    }
    let token = HeaderValue::from_str(&format!("Bearer {}", endpoint.token)).expect("a header");
    out.headers_mut().insert(header::AUTHORIZATION, token);
    if let Some(base) = public_base(&parts.headers, id).and_then(|b| HeaderValue::from_str(&b).ok())
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
    if !host_token_ok(&web, &headers, query.as_deref()) {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong host token");
    }
    let endpoint = match reach(&web, &id).await {
        Ok(endpoint) => endpoint,
        Err(refused) => return *refused,
    };
    // The client's own token is dropped; everything else it said about
    // itself (device, connection, name, kind) is the daemon's to read.
    let mut pairs: Vec<&str> = query
        .as_deref()
        .unwrap_or("")
        .split('&')
        .filter(|kv| !kv.is_empty() && !kv.starts_with("token="))
        .collect();
    let token = format!("token={}", endpoint.token);
    pairs.push(&token);
    let url = format!(
        "{}/events?{}",
        endpoint.base_url.replacen("http", "ws", 1),
        pairs.join("&")
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
    ws.on_upgrade(move |client| tunnel(client, upstream))
}

type UpstreamSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Frames both ways, each direction in order. A send waits for the other
/// side to take the frame, so a slow reader slows the writer instead of the
/// host holding what it cannot deliver. It ends when either side does.
async fn tunnel(client: WebSocket, upstream: UpstreamSocket) {
    let (mut to_client, mut from_client) = client.split();
    let (mut to_daemon, mut from_daemon) = upstream.split();
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
            if to_client.send(frame).await.is_err() || end {
                break;
            }
        }
    };
    tokio::select! { _ = up => {}, _ = down => {} }
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
