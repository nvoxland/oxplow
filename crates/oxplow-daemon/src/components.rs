//! `GET /components/{ext}/{component}/{*path}` (P6b.D3): a private
//! extension's custom component bundle, for the sandboxed frame a
//! `viz: custom` lens renders (`.context/extensions.md`, "Custom
//! components"). The daemon serves it because the shell holds no project
//! state and browser mode has no custom scheme.
//!
//! **Ungated** like `/health`: a frame can't carry the UI token, and what
//! it serves is the extension's own files — never project data, which the
//! frame reaches only through the host's bridged calls. It is outside the
//! permissive CORS layer, so a web page can't read a bundle with `fetch`.
//! Only a loopback `Host` (DNS rebinding: a page whose name resolves to
//! 127.0.0.1 would otherwise read bundles same-origin), only a declared
//! component of an enabled, non-bundled extension, only a plain file
//! inside its bundle folder; every 200 carries a CSP that lets the bundle
//! load its own files and nothing else — no network, no forms.

use std::path::{Component, Path, PathBuf};

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use oxplow_app::extensions::custom_components::MAX_BUNDLE_BYTES;

use crate::DaemonState;

/// The largest single file served (a bundle's whole cap).
pub const MAX_FILE_BYTES: u64 = MAX_BUNDLE_BYTES;

/// The CSP on every bundle file. `sandbox allow-scripts` makes the
/// document's origin opaque however it is loaded — the host's iframe
/// attribute is not the only fence. `source` is the bundle's own folder
/// (`http://<host>/components/<ext>/<component>/`), named beside `'self'`
/// because that origin is opaque; nothing may connect, submit or rebase
/// anywhere. `style-src 'unsafe-inline'` is there because the host sends
/// the kit's CSS and theme tokens as text the bundle injects as a
/// `<style>`; inline *scripts* stay refused.
pub fn bundle_csp(source: &str) -> String {
    let own = format!(" {source}");
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'self'{own}; style-src 'self' 'unsafe-inline'{own}; \
         img-src 'self' data: blob:{own}; font-src 'self' data:{own}; connect-src 'none'; \
         form-action 'none'; base-uri 'none'"
    )
}

/// The `Content-Type` for a bundle file, by extension; unknown types are
/// opaque bytes (with `nosniff`, never run).
pub fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `rel` inside `bundle_root`, when it names a plain file there: no empty,
/// `.`, `..`, backslash or NUL segment, not absolute, not a symlink or a
/// directory, and — resolved — still inside the bundle. An empty `rel` is
/// `index.html`.
pub fn safe_bundle_path(bundle_root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let ok_segment = |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', '\0']);
    if rel.starts_with('/') || !rel.split('/').all(ok_segment) {
        return None;
    }
    let candidate = bundle_root.join(rel);
    if !Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let meta = std::fs::symlink_metadata(&candidate).ok()?;
    if !meta.is_file() {
        return None;
    }
    let root = bundle_root.canonicalize().ok()?;
    let resolved = candidate.canonicalize().ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

/// The request's `Host` when it names this machine's loopback —
/// `127.0.0.1`, `localhost` or `[::1]`, with an optional numeric port.
pub fn loopback_host(headers: &HeaderMap) -> Option<&str> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') => (name, Some(port)),
        _ => (host, None),
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (port_ok && matches!(name, "127.0.0.1" | "localhost" | "[::1]")).then_some(host)
}

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    stream_id: Option<String>,
}

/// `/components/{ext}/{component}`: the folder form, so the bundle's
/// relative URLs resolve inside it.
pub async fn component_root(
    AxumPath((ext, component)): AxumPath<(String, String)>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    if loopback_host(&headers).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let query = q
        .stream_id
        .map(|s| format!("?stream_id={s}"))
        .unwrap_or_default();
    Redirect::permanent(&format!("/components/{ext}/{component}/{query}")).into_response()
}

/// `/components/{ext}/{component}/` — the bundle's `index.html`.
pub async fn component_index(
    state: State<DaemonState>,
    AxumPath((ext, component)): AxumPath<(String, String)>,
    q: Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    serve(state, ext, component, String::new(), q, headers).await
}

/// `/components/{ext}/{component}/{*path}` — one of the bundle's files.
pub async fn component_file(
    state: State<DaemonState>,
    AxumPath((ext, component, path)): AxumPath<(String, String, String)>,
    q: Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    serve(state, ext, component, path, q, headers).await
}

async fn serve(
    State(state): State<DaemonState>,
    ext: String,
    component: String,
    path: String,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(host) = loopback_host(&headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let svc = &state.ctx.services;
    let root = svc.worktrees.resolve(q.stream_id.as_deref()).await;
    let Ok(extension) = svc.extension_catalog.named(&root, &ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if extension.origin == "bundled" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(declared) = extension
        .custom_components
        .iter()
        .find(|c| c.id == component)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bundle_root = root.join(&extension.path).join(&declared.bundle);
    let Some(file) = safe_bundle_path(&bundle_root, &path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::metadata(&file) {
        Ok(m) if m.len() > MAX_FILE_BYTES => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Ok(_) => {}
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    }
    let Ok(bytes) = tokio::fs::read(&file).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let source = format!("http://{host}/components/{ext}/{component}/");
    let mut response = bytes.into_response();
    let h = response.headers_mut();
    let set = |h: &mut HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, content_type_for(&file));
    set(h, header::CONTENT_SECURITY_POLICY, &bundle_csp(&source));
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::CACHE_CONTROL, "no-store");
    set(h, header::REFERRER_POLICY, "no-referrer");
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_path_is_a_plain_file_inside_the_bundle() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("bundle");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "x").unwrap();
        std::fs::write(root.join("assets/app.js"), "x").unwrap();
        std::fs::write(d.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(d.path().join("secret.txt"), root.join("link.txt")).unwrap();
        assert!(safe_bundle_path(&root, "assets/app.js").is_some());
        assert!(safe_bundle_path(&root, "").is_some(), "the index");
        for bad in [
            "../secret.txt",
            "/etc/hosts",
            "assets/../../secret.txt",
            "a\\..\\b",
            "assets",
            "link.txt",
            "assets//app.js",
            "./index.html",
            "nope.js",
        ] {
            assert!(safe_bundle_path(&root, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_csp_sandboxes_the_bundle_itself() {
        let csp = bundle_csp("http://127.0.0.1:1/components/x/c/");
        let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
        assert!(directives.contains(&"sandbox allow-scripts"), "{csp}");
        assert!(directives.contains(&"connect-src 'none'"), "{csp}");
    }

    #[test]
    fn only_a_loopback_host_is_served() {
        let host = |h: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::HOST, HeaderValue::from_str(h).unwrap());
            loopback_host(&m).map(str::to_string)
        };
        for ok in [
            "127.0.0.1",
            "127.0.0.1:7420",
            "localhost:1",
            "[::1]",
            "[::1]:7420",
        ] {
            assert_eq!(host(ok).as_deref(), Some(ok), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:7420",
            "127.0.0.1.evil.example",
            "localhost.evil.example:7420",
            "127.0.0.1:",
            "127.0.0.1:80x",
            "::1",
            "[::1",
            "",
        ] {
            assert_eq!(host(bad), None, "{bad}");
        }
        assert_eq!(loopback_host(&HeaderMap::new()), None, "no Host");
    }

    #[test]
    fn content_types_by_extension() {
        for (name, want) in [
            ("index.html", "text/html; charset=utf-8"),
            ("app.js", "text/javascript; charset=utf-8"),
            ("app.mjs", "text/javascript; charset=utf-8"),
            ("a.css", "text/css; charset=utf-8"),
            ("i.svg", "image/svg+xml"),
            ("i.png", "image/png"),
            ("f.woff2", "font/woff2"),
            ("d.json", "application/json"),
            ("x.bin", "application/octet-stream"),
            ("noext", "application/octet-stream"),
        ] {
            assert_eq!(content_type_for(Path::new(name)), want, "{name}");
        }
    }
}
