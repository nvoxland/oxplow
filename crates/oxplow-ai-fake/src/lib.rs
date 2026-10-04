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
