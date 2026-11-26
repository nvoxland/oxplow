//! The built UI, served from a folder read at run time (`--ui`), never
//! embedded, so a fresh worktree builds without `apps/desktop/dist`.
//!
//! The UI is built with a relative asset base, so the same files load at `/`
//! and under `/p/<project>/`. `/p/<project>/` is the app's `index.html` with
//! `window.__OXPLOW__` put first in its head, the way the Mac shell states it
//! to a window it creates (`oxplow-tauri-ipc` `windows.rs`). `/` is, for now,
//! a plain list of the held projects; the tablet shell replaces it.

use std::path::{Component, Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use oxplow_app::component_http::content_type_for;

use crate::host::DaemonState;
use crate::web::{refuse, Web};

const NO_UI: &str = "this host has no UI folder; start it with --ui <dir>";

/// `window.__OXPLOW__` for the project's window, and the page's frame bound
/// widened to its own origin. Pure.
pub fn project_index(html: &str, origin: &str, project: &str, token: &str) -> String {
    let context = serde_json::json!({
        "kind": "project",
        "base": format!("{origin}/p/{project}"),
        "project": project,
        // The host token, until pairing gives each device its own.
        "token": token,
        "projectDir": null,
    });
    // `<` is escaped so no value can close the script.
    let json = context.to_string().replace('<', "\\u003c");
    let script = format!("<script>window.__OXPLOW__ = Object.freeze({json});</script>");
    // A component's frame is served from this origin, which may be a name
    // other than loopback; the meta CSP in `index.html` must allow it.
    let html = html.replacen("frame-src blob:", &format!("frame-src blob: {origin}"), 1);
    match html.find("<head>") {
        Some(at) => format!("{}{script}{}", &html[..at + 6], &html[at + 6..]),
        None => format!("{script}{html}"),
    }
}

/// The origin a browser reached this request by.
pub(crate) fn origin(scheme: crate::web::Scheme, headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    Some(format!("{}://{host}", scheme.as_str()))
}

pub(crate) async fn project_redirect(Path(id): Path<String>) -> Response {
    Redirect::permanent(&format!("/p/{id}/")).into_response()
}

/// `/p/<project>/`: the app, for a project the registry has.
pub(crate) async fn project_page(
    State(web): State<Web>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if web.host.registry().get(&id).is_none() {
        return refuse(StatusCode::NOT_FOUND, "no such project");
    }
    let Some(dir) = &web.ui_dir else {
        return refuse(StatusCode::NOT_FOUND, NO_UI);
    };
    let Some(origin) = origin(web.scheme, &headers) else {
        return refuse(StatusCode::BAD_REQUEST, "no Host");
    };
    let Ok(index) = tokio::fs::read(dir.join("index.html")).await else {
        return refuse(StatusCode::NOT_FOUND, "the UI folder has no index.html");
    };
    let html = project_index(&String::from_utf8_lossy(&index), &origin, &id, &web.token);
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// `/p/<project>/<file>`: a file of the UI (the app asks for them relative to
/// its page).
pub(crate) async fn project_file(web: &Web, id: &str, rest: &str) -> Response {
    if web.host.registry().get(id).is_none() {
        return refuse(StatusCode::NOT_FOUND, "no such project");
    }
    file(web, rest).await
}

/// Any other path: a file of the UI at the root.
pub(crate) async fn root_file(State(web): State<Web>, uri: Uri) -> Response {
    // A path that is not percent-decoded cannot name `..` by `%2e%2e`, so the
    // decoded form is what is checked.
    let decoded = percent_decode(uri.path().trim_start_matches('/'));
    file(&web, &decoded).await
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `rest` under the UI folder when it names a file inside it: plain names
/// only, so nothing climbs out and nothing absolute is read.
fn inside(dir: &FsPath, rest: &str) -> Option<PathBuf> {
    let rel = FsPath::new(rest);
    let plain = !rest.is_empty()
        && !rest.contains('\\')
        && !rest.split('/').any(str::is_empty)
        && rel.components().all(|c| matches!(c, Component::Normal(_)));
    plain.then(|| dir.join(rel))
}

async fn file(web: &Web, rest: &str) -> Response {
    let Some(dir) = &web.ui_dir else {
        return refuse(StatusCode::NOT_FOUND, NO_UI);
    };
    let Some(path) = inside(dir, rest) else {
        return refuse(StatusCode::NOT_FOUND, "no such file");
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            // Built files are named by their content; anything else may change
            // with the next build.
            let cache = if rest.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, content_type(&path)),
                    (header::CACHE_CONTROL, cache),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => refuse(StatusCode::NOT_FOUND, "no such file"),
    }
}

fn content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        // The browser compiles wasm only as streamed with this type.
        Some("wasm") => "application/wasm",
        Some("map") => "application/json",
        _ => content_type_for(path),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `/`: the held projects, each a link to its page. Names and states only:
/// the page is open to whoever may reach the host.
pub(crate) async fn launcher(State(web): State<Web>) -> Response {
    let held: Vec<_> = web
        .host
        .projects()
        .into_iter()
        .filter(|p| p.holds > 0)
        .collect();
    let items: String = held
        .iter()
        .map(|p| {
            let state = match &p.state {
                DaemonState::Starting => "starting",
                DaemonState::Running => "running",
                DaemonState::Stopping => "stopping",
                DaemonState::Stopped => "stopped",
                DaemonState::Crashed { .. } => "crashed",
            };
            format!(
                "<li><a href=\"/p/{id}/\">{name}</a> <small>{state}</small></li>",
                id = escape(&p.id),
                name = escape(&p.name),
            )
        })
        .collect();
    let body = if held.is_empty() {
        "<p>No project is held.</p>".to_string()
    } else {
        format!("<ul>{items}</ul>")
    };
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>oxplow</title></head><body><h1>oxplow</h1>{body}</body></html>"
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page reached over the TLS listener says `https` in its base, so
    /// its calls and socket stay secure.
    #[test]
    fn the_origin_carries_the_listeners_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "mac.tailnet.ts.net:7400".parse().unwrap());
        assert_eq!(
            origin(crate::web::Scheme::Https, &headers).as_deref(),
            Some("https://mac.tailnet.ts.net:7400")
        );
        assert_eq!(
            origin(crate::web::Scheme::Http, &headers).as_deref(),
            Some("http://mac.tailnet.ts.net:7400")
        );
    }

    #[test]
    fn a_value_cannot_close_the_script() {
        let html = project_index("<head></head>", "http://h", "p</script><b>", "t");
        assert_eq!(html.matches("</script>").count(), 1, "{html}");
        assert!(html.contains("\\u003c/script>"), "{html}");
    }

    #[test]
    fn a_page_with_no_head_gets_the_context_before_everything() {
        let html = project_index("<body>x</body>", "http://h", "p", "t");
        assert!(html.starts_with("<script>window.__OXPLOW__"), "{html}");
    }

    #[test]
    fn only_plain_names_inside_the_folder() {
        let dir = FsPath::new("/ui");
        assert_eq!(
            inside(dir, "assets/a.js"),
            Some(PathBuf::from("/ui/assets/a.js"))
        );
        for bad in [
            "",
            "..",
            "../x",
            "a/../x",
            "/etc/passwd",
            "a//b",
            "a\\b",
            "a/",
        ] {
            assert_eq!(inside(dir, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn percent_escapes_are_decoded_before_the_check() {
        assert_eq!(percent_decode("%2e%2e/x"), "../x");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
