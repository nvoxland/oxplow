//! A local stand-in for an AI provider's HTTP API, for the tests of
//! `oxplow-ai` and the crates that use it (a dev-dependency only).

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use serde_json::Value;

/// Every request the mock received: path, headers, JSON body.
pub type Seen = Arc<Mutex<Vec<(String, HeaderMap, Value)>>>;

/// Serve `reply` (with `status`) to every POST on `path`, recording what
/// arrived. Returns the base URL and the record.
pub async fn mock(path: &'static str, status: u16, reply: Value) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route(
            path,
            post(
                move |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| {
                    let reply = reply.clone();
                    async move {
                        seen.lock().expect("mock record poisoned").push((
                            path.to_string(),
                            headers,
                            body,
                        ));
                        (
                            axum::http::StatusCode::from_u16(status).expect("a valid HTTP status"),
                            Json(reply),
                        )
                    }
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let addr = listener.local_addr().expect("local address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("mock server") });
    (format!("http://{addr}"), seen)
}

/// Serve `chunks` as a `text/event-stream` reply to every POST on `path`,
/// one chunk every `every` (a chunk is raw text: one event, several, or
/// part of one). After the last chunk the stream stays open for `hold`
/// before it ends, so a reader can be cancelled mid-stream. Returns the
/// base URL, the record, and how many chunks were written before the
/// client went away.
pub async fn mock_sse(
    path: &'static str,
    chunks: Vec<String>,
    every: std::time::Duration,
    hold: std::time::Duration,
) -> (String, Seen, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let seen: Seen = Arc::default();
    let written = Arc::new(AtomicUsize::new(0));
    let counter = written.clone();
    let app = Router::new()
        .route(
            path,
            post(
                move |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| {
                    let chunks = chunks.clone();
                    let counter = counter.clone();
                    async move {
                        seen.lock().expect("mock record poisoned").push((
                            path.to_string(),
                            headers,
                            body,
                        ));
                        let total = chunks.len();
                        let stream = futures::stream::unfold(0usize, move |at| {
                            let chunks = chunks.clone();
                            let counter = counter.clone();
                            async move {
                                if at > total {
                                    return None;
                                }
                                if at == total {
                                    tokio::time::sleep(hold).await;
                                    return None;
                                }
                                if at > 0 {
                                    tokio::time::sleep(every).await;
                                }
                                counter.fetch_add(1, Ordering::SeqCst);
                                Some((
                                    Ok::<_, std::convert::Infallible>(chunks[at].clone()),
                                    at + 1,
                                ))
                            }
                        });
                        (
                            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                            axum::body::Body::from_stream(stream),
                        )
                    }
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let addr = listener.local_addr().expect("local address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("mock server") });
    (format!("http://{addr}"), seen, written)
}

/// Serve each POST on `path` the next of `rounds` as a `text/event-stream`
/// reply, all at once (the last repeats): a model asked again after a
/// tool round answers differently. Returns the base URL and the record.
pub async fn mock_sse_rounds(path: &'static str, rounds: Vec<Vec<String>>) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route(
            path,
            post(
                move |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| {
                    let rounds = rounds.clone();
                    async move {
                        let at = {
                            let mut seen = seen.lock().expect("mock record poisoned");
                            seen.push((path.to_string(), headers, body));
                            seen.len() - 1
                        };
                        let round = rounds
                            .get(at)
                            .or(rounds.last())
                            .cloned()
                            .unwrap_or_default();
                        (
                            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                            round.concat(),
                        )
                    }
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let addr = listener.local_addr().expect("local address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("mock server") });
    (format!("http://{addr}"), seen)
}
