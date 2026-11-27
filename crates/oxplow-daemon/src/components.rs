//! `GET /components/v/{version}/{*path}` and `/component-lib/{file}`: a
//! custom component's bundle and the client library, for the sandboxed
//! frame a page renders (`.context/extensions-components.md`, "Custom components").
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

/// The header a host in front of the daemon names its public base in
/// (`http://<name>/p/<project>`): the origin and prefix a browser reaches
/// bundles by.
pub const PUBLIC_BASE_HEADER: &str = "oxplow-public-base";

/// Where this request's bundles are reached from outside, when it may be
/// served: the public base a host says (believed only with the daemon's own
/// token, which only the host has), else the loopback `Host`.
fn reach(headers: &HeaderMap, token: &str) -> Option<String> {
    let from_host = headers
        .get(PUBLIC_BASE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(component_http::public_base)
        .filter(|_| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .is_some_and(|t| crate::same_secret(t, token))
        });
    if let Some(base) = from_host {
        return Some(base.to_string());
    }
    headers
        .get(header::HOST)?
        .to_str()
        .ok()
        .and_then(component_http::loopback_host)
        .map(|host| format!("http://{host}"))
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
/// stylesheet, to a loopback `Host` or the host in front only.
pub async fn component_lib(
    State(state): State<DaemonState>,
    AxumPath(file): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    match (
        reach(&headers, &state.token),
        component_http::library(&file),
    ) {
        (Some(_), Some(served)) => respond(served),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `/components/v/{version}`: the folder form, so the bundle's relative
/// URLs resolve inside it. The redirect appends `/` to the raw request path
/// — decoded segments are never re-formatted.
pub async fn component_root(
    State(state): State<DaemonState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if reach(&headers, &state.token).is_none() {
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
    let Some(base) = reach(&headers, &state.token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match component_http::bundle_file(
        &state.ctx.services.component_bundles,
        &base,
        &version,
        &path,
    )
    .await
    {
        Some(served) => respond(served),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
