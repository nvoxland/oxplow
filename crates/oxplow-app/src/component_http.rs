//! What a custom component's frame is served, whoever serves it: the
//! daemon (`/components/v/{version}/…`, `/component-lib/…`) and a test's
//! harness serve the same bytes under the same headers, so a component
//! tested is the component that runs (`.context/extensions.md`, "Custom
//! components").
//!
//! **What it serves is a deployment.** The host loads the component's
//! bundle (`load_component`) into `Services::component_bundles`, keyed by
//! its version — the component's approval hash over its files — and the
//! frame is served from that bundle's folder in its deployment, which never
//! changes, so it runs what was hashed. Only a loopback `Host` (DNS
//! rebinding: a page whose name resolves to 127.0.0.1 would otherwise read
//! bundles same-origin), only a loaded version, only a file of it; every
//! file carries a CSP that lets the bundle load its own files and the
//! client library and nothing else — no network, no forms.

use std::path::Path;

use crate::component_bundles::ComponentBundles;

/// Where the component client library is served: a folder of its own,
/// beside `/components/` (whose routes would read a path under it as a
/// bundle's).
pub const LIB_PATH: &str = "/component-lib/";

/// The CSP on every bundle file. `lib` is the client library's folder
/// ([`LIB_PATH`]): a script source (the library) and a style source (the
/// kit's sheet), nothing else. `sandbox allow-scripts allow-forms` makes the
/// document's origin opaque however it is loaded — the host's iframe
/// attribute is not the only fence — and lets it hold a form, whose
/// submission goes nowhere (`form-action 'none'`): a component handles
/// `submit` in its script. `source` is the loaded snapshot's own folder
/// (`http://<host>/components/v/<version>/`), named: there is no `'self'`
/// anywhere, since in a sandboxed frame it still matches the server's whole
/// origin — the response URL's — and would let a frame load another
/// bundle's files, which its approval never covered. Nothing may connect,
/// submit or rebase anywhere. A bundle's own inline styles are allowed
/// (`'unsafe-inline'` in `style-src`): CSS here can fetch nothing from
/// outside, since every fetching directive is bounded to the bundle. Inline
/// *scripts* stay refused — `check_components` says so at check, since the
/// frame won't.
pub fn bundle_csp(source: &str, lib: &str) -> String {
    format!(
        "sandbox allow-scripts allow-forms; default-src 'none'; script-src {source} {lib}; style-src 'unsafe-inline' {source} {lib}; \
         img-src data: blob: {source}; font-src data: {source}; connect-src 'none'; \
         form-action 'none'; base-uri 'none'"
    )
}

/// The client library a bundle loads to talk to its host: a classic
/// script defining `oxplow.connect()`. A module would be fetched with CORS,
/// which a sandboxed frame's opaque origin never passes and nothing served
/// to a frame allows.
pub const LIB_JS: &str = include_str!("../assets/component/oxplow-component.js");
/// Its types, for bundle authors.
pub const LIB_TYPES: &str = include_str!("../assets/component/oxplow-component.d.ts");
/// The kit's stylesheet: classes over the theme's tokens, which the
/// library's `applyTheme()` sets on the frame's root.
pub const KIT_CSS: &str = include_str!("../assets/component/oxplow-kit.css");

/// A response: its body, content type and the headers it carries besides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub body: Vec<u8>,
    pub content_type: &'static str,
    /// `(name, value)`, lowercase names.
    pub headers: Vec<(&'static str, String)>,
}

/// `file` of the client library (`oxplow-component.js`, its `.d.ts`, the
/// kit's `oxplow-kit.css`); `None` for anything else.
pub fn library(file: &str) -> Option<Served> {
    let (body, content_type) = match file {
        "oxplow-component.js" => (LIB_JS, "text/javascript; charset=utf-8"),
        "oxplow-component.d.ts" => (LIB_TYPES, "text/plain; charset=utf-8"),
        "oxplow-kit.css" => (KIT_CSS, "text/css; charset=utf-8"),
        _ => return None,
    };
    Some(Served {
        body: body.as_bytes().to_vec(),
        content_type,
        headers: vec![
            ("x-content-type-options", "nosniff".into()),
            // It changes only with the app, which a revalidation notices.
            ("cache-control", "no-cache".into()),
            ("referrer-policy", "no-referrer".into()),
        ],
    })
}

/// `path` (`""`: `index.html`) of the bundle loaded at `version`, exactly
/// one of its files — a name it lists, so nothing else can be named — read
/// from its deployment on disk, for a request whose `Host` is `host` (a
/// loopback one, [`loopback_host`]). `None` when there is no such file.
pub async fn bundle_file(
    bundles: &ComponentBundles,
    host: &str,
    version: &str,
    path: &str,
) -> Option<Served> {
    let snapshot = bundles.get(version)?;
    let rel = path.to_string();
    let body = tokio::task::spawn_blocking(move || snapshot.file(&rel))
        .await
        .ok()??;
    let name = if path.is_empty() { "index.html" } else { path };
    let source = format!("http://{host}/components/v/{version}/");
    let lib = format!("http://{host}{LIB_PATH}");
    Some(Served {
        body,
        content_type: content_type_for(Path::new(name)),
        headers: vec![
            ("content-security-policy", bundle_csp(&source, &lib)),
            ("x-content-type-options", "nosniff".into()),
            ("cache-control", "no-store".into()),
            ("referrer-policy", "no-referrer".into()),
        ],
    })
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

/// `host` (a request's `Host`) when it names this machine's loopback —
/// `127.0.0.1` or `localhost`, with an optional numeric port. Not `[::1]`:
/// the page's `frame-src` can't name an IPv6 literal, so a frame there
/// could never load.
pub fn loopback_host(host: &str) -> Option<&str> {
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') => (name, Some(port)),
        _ => (host, None),
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (port_ok && matches!(name, "127.0.0.1" | "localhost")).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_csp_sandboxes_the_bundle_itself() {
        let csp = bundle_csp(
            "http://127.0.0.1:1/components/x/c/",
            "http://127.0.0.1:1/component-lib/",
        );
        let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
        assert!(
            directives.contains(&"sandbox allow-scripts allow-forms"),
            "{csp}"
        );
        assert!(directives.contains(&"connect-src 'none'"), "{csp}");
        // A form may be there, but never submit anywhere.
        assert!(directives.contains(&"form-action 'none'"), "{csp}");
    }

    /// A bundle may load its own files and the client library, named — and
    /// nothing else on the server. `'self'` would match the server's whole
    /// origin (the response URL's, even in a sandboxed frame — checked in
    /// Chromium and WebKit), so a frame could load another bundle's code,
    /// which its approval never covered.
    #[test]
    fn the_csp_names_the_client_library_beside_the_bundle() {
        let csp = bundle_csp(
            "http://127.0.0.1:1/components/x/c/",
            "http://127.0.0.1:1/component-lib/",
        );
        let script = csp
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("script-src"))
            .unwrap();
        assert_eq!(
            script,
            "script-src http://127.0.0.1:1/components/x/c/ http://127.0.0.1:1/component-lib/"
        );
        assert!(!csp.contains("'self'"), "{csp}");
        // And the kit's stylesheet, beside it — no image or font.
        let style = csp
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("style-src"))
            .unwrap();
        assert!(
            style.ends_with(" http://127.0.0.1:1/component-lib/"),
            "{style}"
        );
        assert_eq!(csp.matches("/component-lib/").count(), 2, "{csp}");
    }

    #[test]
    fn only_a_loopback_host_is_served() {
        for ok in ["127.0.0.1", "127.0.0.1:7420", "localhost:1"] {
            assert_eq!(loopback_host(ok), Some(ok), "{ok}");
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
            // No CSP can bound a frame to an IPv6 literal, so a bundle is
            // never served there.
            "[::1]",
            "[::1]:7420",
            "",
        ] {
            assert_eq!(loopback_host(bad), None, "{bad}");
        }
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

    #[test]
    fn the_library_serves_its_three_files_and_nothing_else() {
        for file in [
            "oxplow-component.js",
            "oxplow-component.d.ts",
            "oxplow-kit.css",
        ] {
            assert!(library(file).is_some_and(|s| !s.body.is_empty()), "{file}");
        }
        assert_eq!(library("other.js"), None);
    }
}
