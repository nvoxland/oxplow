//! `GET /components/v/{version}/{*path}` and `/component-lib/{file}`: a
//! custom component's bundle and the client library, for the sandboxed
//! frame a page renders (`.context/extensions.md`, "Custom components").
//! The daemon serves them because the shell holds no project state and
//! browser mode has no custom scheme. What is served, and under which
//! headers, is `oxplow_app::component_http`'s — the same a test's harness
//! serves.
//!
//! **Ungated** like `/health`: a frame can't carry the UI token, and what
//! it serves is an extension's own files — never project data, which the
//! frame reaches only through the host's bridged calls. It is outside the
//! permissive CORS layer, so a web page can't read a bundle with `fetch`.

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Redirect, Response},
};
use oxplow_app::component_http::{self, Served};

use crate::DaemonState;

pub use oxplow_app::component_http::LIB_PATH;

/// The request's `Host`, when it names this machine's loopback.
fn loopback(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::HOST)?
        .to_str()
        .ok()
        .and_then(component_http::loopback_host)
}

/// `served` as a response.
fn respond(served: Served) -> Response {
    let mut response = served.body.into_response();
    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(served.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    for (name, value) in served.headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            h.insert(name, value);
        }
    }
    response
}

/// `/component-lib/{file}` — the client library, its types or the kit's
/// stylesheet, to a loopback `Host` only.
pub async fn component_lib(AxumPath(file): AxumPath<String>, headers: HeaderMap) -> Response {
    match (loopback(&headers), component_http::library(&file)) {
        (Some(_), Some(served)) => respond(served),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `/components/v/{version}`: the folder form, so the bundle's relative
/// URLs resolve inside it. The redirect appends `/` to the raw request path
/// — decoded segments are never re-formatted.
pub async fn component_root(uri: Uri, headers: HeaderMap) -> Response {
    if loopback(&headers).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Redirect::permanent(&format!("{}/", uri.path())).into_response()
}

/// `/components/v/{version}/` — the bundle's `index.html`.
pub async fn component_index(
    state: State<DaemonState>,
    AxumPath(version): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    serve(state, version, String::new(), headers).await
}

/// `/components/v/{version}/{*path}` — one of the bundle's files.
pub async fn component_file(
    state: State<DaemonState>,
    AxumPath((version, path)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve(state, version, path, headers).await
}

async fn serve(
    State(state): State<DaemonState>,
    version: String,
    path: String,
    headers: HeaderMap,
) -> Response {
    let Some(host) = loopback(&headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match component_http::bundle_file(&state.ctx.services.component_bundles, host, &version, &path)
        .await
    {
        Some(served) => respond(served),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
