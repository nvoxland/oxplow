//! `custom_components:` (P6b.D1; **stable since P11**, tsk962): a web
//! bundle a `viz: custom` lens renders in a sandboxed frame. The frame has
//! no origin, no network and no daemon token, so it reaches only what it
//! declares: the lenses it may query (`assets`, lens ids) and the commands
//! it may invoke (`commands`). It renders and queries without approval;
//! one that declares `commands` is a program a person approves before its
//! `invoke` runs them with their rights (tsk960), at the version of the
//! bundle it loaded (tsk984). A bundled extension can't declare one: its
//! bundle is never served.
//!
//! ```yaml
//! custom_components:
//!   - id: burndown
//!     title: Burndown
//!     bundle: components/burndown        # default components/<id>; holds index.html
//!     assets: [open-tasks, oxplow-bundled/visits]   # a bare slug is this extension's
//!     commands: [oxplow.work_item.transition]
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, entry_line, key_line};

/// The most a bundle may hold.
pub const MAX_BUNDLE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_BUNDLE_FILES: usize = 256;

/// A declared component (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomComponent {
    /// `[a-z0-9-]+`, unique in the extension; a lens names it in
    /// `custom.component`.
    pub id: String,
    pub extension: String,
    pub title: Option<String>,
    /// The bundle's folder inside the extension, holding `index.html`.
    pub bundle: String,
    /// Lens ids (`<extension>/<slug>`) the component may query.
    pub assets: Vec<String>,
    /// Commands the component may invoke.
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentFile {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    bundle: Option<String>,
    #[serde(default)]
    assets: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
}

/// What a bundle folder holds, as far as loading cares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleStat {
    pub has_index: bool,
    pub files: usize,
    pub bytes: u64,
    /// A symlink found inside it (its path in the bundle), if any.
    pub symlink: Option<String>,
}

/// What looking a bundle up found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleLook {
    /// It isn't there (on disk: not a folder).
    Absent,
    /// It can't be told from these files: a revision's tree, where a built
    /// bundle usually isn't committed. Taken as declared (tsk784).
    Unknown,
    Found(BundleStat),
}

/// What files under `rel` say about a bundle, for a tree of paths and
/// contents (a revision): their count, size and an `index.html`; `Unknown`
/// when none are there.
pub fn look_in_paths<'a>(rel: &str, files: impl Iterator<Item = (&'a str, usize)>) -> BundleLook {
    let prefix = format!("{}/", rel.trim_end_matches('/'));
    let mut stat = BundleStat::default();
    for (path, len) in files {
        let Some(inner) = path.strip_prefix(&prefix) else {
            continue;
        };
        stat.files += 1;
        stat.bytes += len as u64;
        if inner == "index.html" {
            stat.has_index = true;
        }
    }
    if stat.files == 0 {
        BundleLook::Unknown
    } else {
        BundleLook::Found(stat)
    }
}

/// Walk `dir` (a bundle on disk): `None` when it isn't a directory. A
/// symlink anywhere — the folder itself included — is reported, never
/// followed.
pub fn stat_bundle(dir: &Path) -> Option<BundleStat> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.file_type().is_symlink() {
        return Some(BundleStat {
            symlink: Some(".".into()),
            ..BundleStat::default()
        });
    }
    if !meta.is_dir() {
        return None;
    }
    let mut stat = BundleStat {
        has_index: std::fs::symlink_metadata(dir.join("index.html")).is_ok_and(|m| m.is_file()),
        ..BundleStat::default()
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                stat.symlink = Some(
                    path.strip_prefix(dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
                return Some(stat);
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                stat.files += 1;
                stat.bytes += meta.len();
            }
        }
    }
    Some(stat)
}

/// The client library's path, as a bundle's `index.html` loads it.
pub const LIB_SCRIPT: &str = "/component-lib/oxplow-component.js";

/// What `ext`'s components ask for that their frame would refuse without a
/// word (tsk961), as check errors at the file: each bundle's `index.html`
/// ([`page_problems`]) — the one page a frame shows, since a nested frame
/// is refused and navigating away ends the component — read through
/// `read`, the files of the extension under check (tsk994: a candidate
/// under review, a revision, or the folder on disk); and any component in
/// an extension that comes with oxplow, whose bundle is never served. A
/// page `read` can't find (a revision's tree rarely holds a built bundle)
/// is skipped: loading says when a bundle has no `index.html`.
pub fn bundle_problems(
    ext: &super::Extension,
    read: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let dir = ext.path.trim_end_matches('/');
    if ext.origin == "bundled" {
        return ext
            .custom_components
            .iter()
            .map(|c| {
                format!(
                    "{dir}/extension.yaml: custom component `{}`: an extension that comes with \
                     oxplow can't declare one — its bundle is never served",
                    c.id
                )
            })
            .collect();
    }
    let mut out = Vec::new();
    for c in &ext.custom_components {
        let rel = format!("{}/index.html", c.bundle.trim_end_matches('/'));
        let Some(html) = read(&rel) else {
            continue;
        };
        out.extend(
            page_problems(&html, true)
                .into_iter()
                .map(|p| format!("{dir}/{rel}: {p}")),
        );
    }
    out
}

/// What one of a bundle's pages asks for that the bundle's CSP refuses
/// silently or that ends the component: an inline `<script>` or event
/// handler, a `type="module"` script, a load from outside the bundle
/// (`src`, `srcset`, a stylesheet), a `<base>`, a nested frame, a plugin,
/// a form, a refresh; and, for its `index.html`, never loading the client
/// library. A link (`<a href>`) is a navigation, not a load, and isn't one.
pub fn page_problems(html: &str, index: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut loads_lib = false;
    let outside_bundle = |url: &str| {
        format!(
            "`{url}` is outside the bundle — the frame's CSP loads only the bundle's own \
             files and oxplow's `/component-lib/`"
        )
    };
    for tag in tags(html) {
        let attr = |n: &str| {
            tag.attrs
                .iter()
                .find(|(k, _)| k == n)
                .map(|(_, v)| v.as_str())
        };
        // A custom element's `on…` attribute is its own (`once`, `only`);
        // on a standard element it's an event handler.
        if !tag.name.contains('-') {
            for (name, _) in &tag.attrs {
                if name.len() > 2
                    && name.starts_with("on")
                    && name[2..].chars().all(|c| c.is_ascii_alphabetic())
                {
                    out.push(format!(
                        "an inline event handler (`{name}`) — the frame's CSP refuses it; add \
                         the listener from a script file"
                    ));
                }
            }
        }
        let refused = match tag.name.as_str() {
            "base" => Some("a <base> — the frame's CSP refuses one (`base-uri 'none'`)"),
            "iframe" | "frame" => Some("a <iframe> — the frame's CSP refuses a nested frame"),
            "object" => Some("an <object> — the frame's CSP refuses plugins"),
            "embed" => Some("an <embed> — the frame's CSP refuses plugins"),
            "form" => {
                Some("a <form> — the frame's CSP refuses submitting one (`form-action 'none'`)")
            }
            "meta"
                if attr("http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh")) =>
            {
                Some("a refresh — it navigates the frame away, which ends the component")
            }
            _ => None,
        };
        out.extend(refused.map(str::to_string));
        let loaded = match tag.name.as_str() {
            "link" => attr("href"),
            "a" | "base" | "iframe" | "frame" | "object" | "embed" => None,
            _ => attr("src"),
        };
        if let Some(url) = loaded {
            let path = url.trim().split(['?', '#']).next().unwrap_or_default();
            if path == LIB_SCRIPT && tag.name == "script" {
                loads_lib = true;
            }
            // An image or font may be a `data:` URL; a script or sheet can't.
            if outside(url, !matches!(tag.name.as_str(), "script" | "link")) {
                out.push(outside_bundle(url));
            }
        }
        for candidate in attr("srcset").unwrap_or_default().split(',') {
            let url = candidate.split_whitespace().next().unwrap_or_default();
            if !url.is_empty() && outside(url, true) {
                out.push(outside_bundle(url));
            }
        }
        if tag.name == "script" {
            let kind = attr("type").map(|t| t.trim().to_ascii_lowercase());
            if kind.as_deref() == Some("module") {
                out.push(
                    "a `type=\"module\"` script — a sandboxed frame can't load modules; use a \
                     classic script"
                        .into(),
                );
            }
            // A script of another type is a data block (JSON, a template):
            // it never runs, so the CSP has nothing to refuse.
            let runs = matches!(
                kind.as_deref(),
                None | Some("" | "text/javascript" | "application/javascript")
            );
            if runs && attr("src").is_none() && !tag.body.trim().is_empty() {
                out.push(
                    "an inline <script> — the frame's CSP runs only script files; move it into a \
                     `.js` file"
                        .into(),
                );
            }
        }
    }
    if index && !loads_lib {
        out.push(format!(
            "doesn't load the client library (`<script src=\"{LIB_SCRIPT}\"></script>`), which \
             is how a component talks to oxplow"
        ));
    }
    out
}

/// A URL the frame would fetch from somewhere other than its own bundle
/// folder or oxplow's `/component-lib/` (tsk983): one with a scheme
/// (`data:` too, for a script or stylesheet — `data` says when it's a
/// load that may take one), a protocol-relative `//host`, an absolute path
/// elsewhere on the daemon, or a relative one whose `..` climbs out of the
/// bundle.
fn outside(url: &str, data: bool) -> bool {
    let url = url.trim();
    let path = url.split(['?', '#']).next().unwrap_or_default();
    if path.starts_with("//") {
        return true;
    }
    if let Some((scheme, _)) = path.split_once(':') {
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            return !(data && scheme.eq_ignore_ascii_case("data"));
        }
    }
    if let Some(rest) = path.strip_prefix('/') {
        return !rest.starts_with("component-lib/");
    }
    let mut depth = 0i32;
    for segment in path.split('/') {
        match segment {
            ".." => depth -= 1,
            "" | "." => {}
            _ => depth += 1,
        }
        if depth < 0 {
            return true;
        }
    }
    false
}

/// One start tag of a page: its lowercased name, its attributes
/// (lowercased names, unquoted values) and, for a raw-text element
/// (`<script>`, `<style>`, `<textarea>`, `<title>`), its text.
struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    body: String,
}

/// The start tags of `html`, comments left out — enough to see what a page
/// loads and runs, not a full HTML parser.
fn tags(html: &str) -> Vec<Tag> {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = html[i..].find('<').map(|p| p + i) {
        if html[at..].starts_with("<!--") {
            i = html[at..].find("-->").map_or(html.len(), |e| at + e + 3);
            continue;
        }
        let mut j = at + 1;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
            j += 1;
        }
        if j == at + 1 {
            i = at + 1;
            continue;
        }
        let name = lower[at + 1..j].to_string();
        let mut attrs = Vec::new();
        loop {
            while j < bytes.len() && (bytes[j].is_ascii_whitespace() || bytes[j] == b'/') {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] == b'>' {
                break;
            }
            let start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() && !b"=>/".contains(&bytes[j])
            {
                j += 1;
            }
            let key = lower[start..j].to_string();
            let mut value = String::new();
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                match bytes.get(j) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let end = html[j + 1..]
                            .find(q as char)
                            .map_or(html.len(), |e| j + 1 + e);
                        value = html[j + 1..end].to_string();
                        j = (end + 1).min(html.len());
                    }
                    _ => {
                        let start = j;
                        while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b'>'
                        {
                            j += 1;
                        }
                        value = html[start..j].to_string();
                    }
                }
            }
            // An empty name is a stray `=`, its value consumed above.
            if !key.is_empty() {
                attrs.push((key, value));
            }
        }
        let open_end = (j + 1).min(html.len());
        // A raw-text element's content is text, not tags: skipped to its
        // close (a script's kept, to tell an inline one).
        let body = if matches!(name.as_str(), "script" | "style" | "textarea" | "title") {
            let close = lower[open_end..]
                .find(&format!("</{name}"))
                .map_or(html.len(), |e| open_end + e);
            let text = html[open_end..close].to_string();
            i = close;
            text
        } else {
            i = open_end;
            String::new()
        };
        out.push(Tag { name, attrs, body });
    }
    out
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A relative path inside the extension, without `..`.
fn inside(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Parse `custom_components:`: the valid ones and an error (`file:line:
/// …`) for each broken one. `stat` describes a bundle folder by its path
/// inside the extension (`None`: no such folder).
pub fn parse_custom_components(
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    stat: &dyn Fn(&str) -> BundleLook,
) -> (Vec<CustomComponent>, Vec<String>) {
    let block = key_line(manifest, "custom_components");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`custom_components` must be a list")],
        );
    };
    let kinds = oxplow_domain::refs::kind::core_kinds();
    let mut out: Vec<CustomComponent> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let c: ComponentFile = match serde_yaml::from_value(item.clone()) {
            Ok(c) => c,
            Err(e) => {
                errors.push(at(file, block, format!("custom component: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "custom_components", "id", &c.id).or(block);
        let bundle = c
            .bundle
            .clone()
            .unwrap_or_else(|| format!("components/{}", c.id));
        let assets: Vec<String> = c
            .assets
            .iter()
            .map(|a| {
                if a.contains('/') {
                    a.clone()
                } else {
                    format!("{extension}/{a}")
                }
            })
            .collect();
        let problem = if !valid_id(&c.id) {
            Some(format!(
                "custom component id `{}` must be lowercase letters, digits and dashes",
                c.id
            ))
        } else if out.iter().any(|o| o.id == c.id) {
            Some(format!("custom component `{}` is declared twice", c.id))
        } else if !inside(&bundle) {
            Some(format!(
                "custom component `{}`: bundle `{bundle}` must be a folder inside the extension",
                c.id
            ))
        } else if let Some(bad) = assets.iter().find(|a| {
            // A lens id alone: params, a revision or a fragment would make
            // it a different ref than the one `query(asset)` names.
            a.contains(['?', '@', '#'])
                || oxplow_domain::refs::grammar::CanonicalRef::parse(&format!("lens:{a}"))
                    .map_or(true, |r| kinds.validate(&r).is_err())
        }) {
            Some(format!(
                "custom component `{}`: asset `{bad}` isn't a lens id (`<slug>` or `<extension>/<slug>`)",
                c.id
            ))
        } else if let Some(bad) = c
            .commands
            .iter()
            .find(|n| oxplow_domain::CommandSpec::validate_id(n).is_err())
        {
            Some(format!(
                "custom component `{}`: `{bad}` isn't a command name",
                c.id
            ))
        } else {
            match stat(&bundle) {
                BundleLook::Unknown => None,
                BundleLook::Absent => Some(format!(
                    "custom component `{}`: bundle `{bundle}` isn't a folder in the extension",
                    c.id
                )),
                BundleLook::Found(s) if s.symlink.is_some() => Some(format!(
                    "custom component `{}`: bundle `{bundle}` holds a symlink (`{}`); a bundle is \
                     plain files",
                    c.id,
                    s.symlink.unwrap_or_default()
                )),
                BundleLook::Found(s) if !s.has_index => Some(format!(
                    "custom component `{}`: bundle `{bundle}` has no `index.html`",
                    c.id
                )),
                BundleLook::Found(s)
                    if s.files > MAX_BUNDLE_FILES || s.bytes > MAX_BUNDLE_BYTES =>
                {
                    Some(format!(
                        "custom component `{}`: bundle `{bundle}` is {} files, {} bytes; the most \
                         is {MAX_BUNDLE_FILES} files, {MAX_BUNDLE_BYTES} bytes",
                        c.id, s.files, s.bytes
                    ))
                }
                BundleLook::Found(_) => None,
            }
        };
        match problem {
            Some(p) => errors.push(at(file, line, p)),
            None => out.push(CustomComponent {
                id: c.id,
                extension: extension.to_string(),
                title: c.title,
                bundle,
                assets,
                commands: c.commands,
            }),
        }
    }
    (out, errors)
}

#[cfg(test)]
mod tests {

    /// tsk784: a revision's tree loads a custom component cleanly — checked
    /// from the bundle's paths when the tree holds them, and taken as is
    /// when it doesn't (a built bundle usually isn't committed). A bundle
    /// that's there but has no `index.html` is still an error.
    #[test]
    fn a_revision_loads_a_custom_component_without_a_bogus_error() {
        let manifest = "manifest: 2\nname: acme\nsharing: private\nintent: { purpose: p, origin: null, examples: [] }\ncustom_components:\n  - { id: chart, bundle: ui/chart }\n";
        let tree = |extra: &[(&str, &str)]| {
            crate::extensions::Tree::new(
                std::iter::once(("extension.yaml".to_string(), manifest.to_string()))
                    .chain(extra.iter().map(|(p, b)| (p.to_string(), b.to_string()))),
            )
            .load("acme", "oxplow/extensions/acme")
        };
        let unbuilt = tree(&[]);
        assert!(unbuilt.errors.is_empty(), "{:?}", unbuilt.errors);
        assert_eq!(unbuilt.custom_components.len(), 1);
        let built = tree(&[
            ("ui/chart/index.html", "<html></html>"),
            ("ui/chart/app.js", "1"),
        ]);
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        let broken = tree(&[("ui/chart/app.js", "1")]);
        assert!(
            broken.errors.join("\n").contains("has no `index.html`"),
            "{:?}",
            broken.errors
        );
    }

    use super::{bundle_problems, page_problems};
    use crate::extensions::{load_extensions, Extension, LensViz};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const LENS: &str = "title: Burn\nquery: SELECT 1 AS day, 3 AS remaining\nviz: custom\ncustom: { component: burndown, props: { color: accent } }\n";

    /// Extension `x` (`sharing`) declaring `components` (the block's
    /// entries), with a `burndown` bundle and the `burn` lens.
    fn load(root: &Path, sharing: &str, components: &str) -> Extension {
        write(
            root,
            "oxplow/extensions/x/extension.yaml",
            &format!(
                "manifest: 2\nname: x\nsharing: {sharing}\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\ncustom_components:\n{components}"
            ),
        );
        write(
            root,
            "oxplow/extensions/x/components/burndown/index.html",
            &format!("<!doctype html>{LIB_TAG}<script src=\"app.js\"></script>"),
        );
        write(root, "oxplow/extensions/x/components/burndown/app.js", "1");
        write(root, "oxplow/extensions/x/lenses/burn.yaml", LENS);
        write(
            root,
            "oxplow/extensions/x/lenses/open-tasks.yaml",
            "title: Open\nquery: SELECT 1\n",
        );
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap()
    }

    const LIB_TAG: &str = "<script src=\"/component-lib/oxplow-component.js\"></script>";

    /// P11 (tsk961): what a bundle's CSP would refuse without a word — an
    /// inline script or event handler, a module, anything from outside the
    /// bundle — is an error at check, and so is an `index.html` that never
    /// loads the client library. The kit's sheet and the bundle's own
    /// files are fine.
    #[test]
    fn a_page_the_csp_would_silently_refuse_is_an_error() {
        let ok = format!(
            "<!doctype html><!-- <script>not code</script> --><link rel=\"stylesheet\" \
             href=\"/component-lib/oxplow-kit.css\"><link rel=stylesheet href=own.css>{LIB_TAG}\
             <script src=\"app.js\"></script><img src=\"data:image/png;base64,AA==\">\
             <a href=\"https://example.com\">a link</a>"
        );
        assert_eq!(page_problems(&ok, true), Vec::<String>::new());
        for (html, says) in [
            ("<script>render()</script>", "an inline <script>"),
            (
                "<script type=\"module\" src=\"app.js\"></script>",
                "`type=\"module\"`",
            ),
            (
                "<script src=\"https://cdn.example/x.js\"></script>",
                "`https://cdn.example/x.js` is outside the bundle",
            ),
            (
                "<link rel=stylesheet href=//cdn.example/x.css>",
                "`//cdn.example/x.css` is outside the bundle",
            ),
            ("<button onclick=\"go()\">Go</button>", "`onclick`"),
            ("<SCRIPT>render()</SCRIPT>", "an inline <script>"),
        ] {
            let problems = page_problems(&format!("{LIB_TAG}{html}"), true).join("\n");
            assert!(problems.contains(says), "{html}: {problems}");
        }
        // tsk983: the frame loads its own folder and the library only — a
        // path that leaves the bundle is refused, so it's reported.
        for (html, says) in [
            (
                "<script src=\"../b/app.js\"></script>",
                "`../b/app.js` is outside the bundle",
            ),
            (
                "<script src=\"/elsewhere/x.js\"></script>",
                "`/elsewhere/x.js` is outside the bundle",
            ),
            (
                "<link rel=stylesheet href=\"lib/../../x.css\">",
                "`lib/../../x.css` is outside the bundle",
            ),
            (
                "<script src=\"data:text/javascript,1\"></script>",
                "`data:text/javascript,1` is outside the bundle",
            ),
        ] {
            let problems = page_problems(&format!("{LIB_TAG}{html}"), true).join("\n");
            assert!(problems.contains(says), "{html}: {problems}");
        }
        for inside in [
            "<script src=\"lib/../app.js\"></script>",
            "<script src=\"./app.js?v=2#x\"></script>",
            "<link rel=stylesheet href=\"/component-lib/oxplow-kit.css\">",
            "<img src=\"data:image/png;base64,AA==\">",
        ] {
            assert_eq!(
                page_problems(&format!("{LIB_TAG}{inside}"), true),
                Vec::<String>::new(),
                "{inside}"
            );
        }
        // tsk994: no panic on an `=` attribute before a multibyte char, and
        // it doesn't swallow the next tag.
        assert_eq!(
            page_problems(&format!("{LIB_TAG}<a =\"x\"é>"), true),
            Vec::<String>::new()
        );
        assert!(
            page_problems(&format!("{LIB_TAG}<a =\"x\"><script>evil()</script>"), true)
                .join("\n")
                .contains("an inline <script>")
        );
        // What the frame runs fine isn't reported…
        for fine in [
            "<script type=\"application/json\" id=data>{\"a\": 1}</script>",
            "<my-chart once only></my-chart>",
            "<style>a::before { content: \"<script>\"; }</style>",
            "<textarea><script>not code</script></textarea>",
            "<title>a <b> title</title>",
        ] {
            assert_eq!(
                page_problems(&format!("{LIB_TAG}{fine}"), true),
                Vec::<String>::new(),
                "{fine}"
            );
        }
        assert_eq!(
            page_problems(
                "<script src=\"/component-lib/oxplow-component.js?v=2\"></script>",
                true
            ),
            Vec::<String>::new(),
            "the library with a query"
        );
        // …and what it refuses is.
        for (html, says) in [
            (
                "<img srcset=\"a.png 1x, https://x.example/b.png 2x\">",
                "`https://x.example/b.png` is outside the bundle",
            ),
            ("<base href=\"x/\">", "a <base>"),
            ("<iframe src=\"other.html\"></iframe>", "a <iframe>"),
            ("<object data=\"x.swf\"></object>", "an <object>"),
            ("<embed src=\"x.swf\">", "an <embed>"),
            ("<form><input></form>", "a <form>"),
            (
                "<meta http-equiv=\"refresh\" content=\"0;url=x\">",
                "a refresh",
            ),
        ] {
            let problems = page_problems(&format!("{LIB_TAG}{html}"), true).join("\n");
            assert!(problems.contains(says), "{html}: {problems}");
        }
        assert!(page_problems("<script src=app.js></script>", true)
            .join("\n")
            .contains("doesn't load the client library"));
        assert_eq!(
            page_problems("<p>another page</p>", false),
            Vec::<String>::new(),
            "only index.html must load it"
        );
    }

    /// tsk994: the lints read the files of the extension being checked —
    /// a candidate under review is not what's installed — through the read
    /// the check is given.
    #[test]
    fn a_bundles_page_is_read_from_the_files_being_checked() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "private", "  - { id: burndown }\n");
        let candidate = |rel: &str| {
            (rel == "components/burndown/index.html")
                .then(|| "<!doctype html><script>render()</script>".to_string())
        };
        let problems = bundle_problems(&ext, &candidate).join("\n");
        assert!(
            problems
                .contains("oxplow/extensions/x/components/burndown/index.html: an inline <script>"),
            "{problems}"
        );
        // A version without the built page (a revision's tree) has nothing
        // to lint: loading says when a bundle has no `index.html`.
        let missing = |_: &str| None;
        assert_eq!(bundle_problems(&ext, &missing), Vec::<String>::new());
    }

    /// tsk961: the check reports a bundle's page problems at the page, and
    /// a component in an extension that comes with oxplow — the daemon
    /// never serves a bundled extension's bundle.
    #[tokio::test]
    async fn a_bundles_pages_and_a_bundled_component_are_checked() {
        let d = tempfile::tempdir().unwrap();
        load(d.path(), "private", "  - { id: burndown }\n");
        write(
            d.path(),
            "oxplow/extensions/x/components/burndown/index.html",
            "<!doctype html><script>render()</script>",
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        let v = crate::extensions::validate_extension(&layer, &cat, d.path(), "x", None)
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("oxplow/extensions/x/components/burndown/index.html: an inline <script>"),
            "{errs}"
        );
        assert!(errs.contains("doesn't load the client library"), "{errs}");
        let mut ext = load(d.path(), "private", "  - { id: burndown }\n");
        ext.origin = "bundled".into();
        assert!(
            bundle_problems(&ext, &|_: &str| None)
                .join("\n")
                .contains("comes with oxplow"),
            "{ext:?}"
        );
    }

    #[test]
    fn a_private_extension_loads_a_component_and_its_lens() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(
            d.path(),
            "private",
            "  - { id: burndown, title: Burndown, assets: [open-tasks, oxplow-bundled/visits], commands: [oxplow.work_item.transition] }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let c = &ext.custom_components[0];
        assert_eq!(c.bundle, "components/burndown");
        assert_eq!(c.assets, vec!["x/open-tasks", "oxplow-bundled/visits"]);
        let lens = ext.lenses.iter().find(|l| l.slug == "burn").unwrap();
        assert_eq!(lens.viz, LensViz::Custom);
        assert_eq!(
            lens.custom.as_ref().unwrap().props.as_ref().unwrap()["color"],
            "accent"
        );
    }

    #[test]
    fn a_broken_component_is_an_error_at_its_line() {
        for (entry, setup, says) in [
            ("{ id: Bad Id }", "", "must be lowercase"),
            (
                "{ id: c, bundle: ../x }",
                "",
                "must be a folder inside the extension",
            ),
            (
                "{ id: c, bundle: nowhere }",
                "",
                "isn't a folder in the extension",
            ),
            ("{ id: c, bundle: empty }", "empty", "has no `index.html`"),
            ("{ id: c, bundle: linked }", "symlink", "holds a symlink"),
            (
                "{ id: c, bundle: \".\" }",
                "",
                "must be a folder inside the extension",
            ),
            (
                "{ id: c, bundle: aliased }",
                "linked-folder",
                "holds a symlink",
            ),
            (
                "{ id: c, bundle: dirindex }",
                "index-dir",
                "has no `index.html`",
            ),
            ("{ id: c, bundle: big }", "big", "the most is 256 files"),
            (
                "{ id: c, bundle: components/burndown, assets: [nope] }",
                "",
                "asset `x/nope` isn't in this extension's lenses/",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"Not A Lens\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"open-tasks?stream_id=2\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"open-tasks#x\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, commands: [Nope] }",
                "",
                "isn't a command name",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            let base = d.path().join("oxplow/extensions/x");
            match setup {
                "empty" => write(d.path(), "oxplow/extensions/x/empty/readme.txt", "x"),
                "symlink" => {
                    write(d.path(), "oxplow/extensions/x/linked/index.html", "x");
                    std::os::unix::fs::symlink("/etc/hosts", base.join("linked/hosts")).unwrap();
                }
                "linked-folder" => {
                    write(d.path(), "oxplow/extensions/x/real/index.html", "x");
                    std::os::unix::fs::symlink(base.join("real"), base.join("aliased")).unwrap();
                }
                "index-dir" => write(
                    d.path(),
                    "oxplow/extensions/x/dirindex/index.html/a.js",
                    "x",
                ),
                "big" => {
                    write(d.path(), "oxplow/extensions/x/big/index.html", "x");
                    for i in 0..256 {
                        write(d.path(), &format!("oxplow/extensions/x/big/f{i}.js"), "x");
                    }
                }
                _ => {}
            }
            let ext = load(
                d.path(),
                "private",
                &format!("  - {entry}\n  - {{ id: burndown }}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert_eq!(
                ext.custom_components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["burndown"],
                "{entry}"
            );
        }
    }

    /// P11 (tsk962): `custom_components` is stable, so a shared extension
    /// declares one like a private one, and its custom lens loads.
    #[test]
    fn a_shared_extension_loads_a_component_and_its_lens() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "shared", "  - { id: burndown }\n");
        assert_eq!(ext.errors, Vec::<String>::new());
        assert_eq!(
            ext.custom_components
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["burndown"]
        );
        assert!(ext.lenses.iter().any(|l| l.slug == "burn"));
    }

    #[test]
    fn a_custom_lens_needs_its_component_and_a_query() {
        for (lens, says) in [
            (
                "title: B\nquery: SELECT 1\nviz: custom\n",
                "needs `custom: { component: <id> }`",
            ),
            (
                "title: B\nviz: custom\ncustom: { component: burndown }\n",
                "needs a `query`",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            write(d.path(), "oxplow/extensions/x/lenses/other.yaml", lens);
            let ext = load(d.path(), "private", "  - { id: burndown }\n");
            assert!(
                ext.errors.join("\n").contains(says),
                "{lens}: {:?}",
                ext.errors
            );
            assert!(ext.lenses.iter().all(|l| l.slug != "other"));
        }
        let spec: crate::extensions::LensSpec = serde_json::from_value(serde_json::json!({
            "title": "B", "query": "SELECT 1", "viz": "custom"
        }))
        .unwrap();
        assert!(
            crate::extensions::spec_problem(&spec).is_some(),
            "an answer can't carry one"
        );
    }

    #[tokio::test]
    async fn validation_checks_its_commands_reads_as_a_table_and_nudges_a_lookalike() {
        let d = tempfile::tempdir().unwrap();
        load(
            d.path(),
            "private",
            "  - { id: burndown, commands: [nope.no.cmd, oxplow.work_item.transition] }\n",
        );
        write(
            d.path(),
            "oxplow/extensions/x/lenses/burn.yaml",
            "title: Burn\nquery: SELECT 1 AS day, 3 AS remaining\nviz: custom\ncustom: { component: burndown }\nchart: { x: day, y: remaining }\n",
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        let schema = |n: &str| {
            (n == "oxplow.work_item.transition").then(|| serde_json::json!({ "type": "object" }))
        };
        let v = crate::extensions::validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("declares command `nope.no.cmd`, which isn't registered"),
            "{errs}"
        );
        assert!(!errs.contains("oxplow.work_item.transition"), "{errs}");
        assert!(
            v.warnings
                .join("\n")
                .contains("the kit's `chart` viz may already cover it"),
            "{:?}",
            v.warnings
        );
        let run = crate::extensions::run_lens(
            &layer,
            &cat,
            d.path(),
            "x/burn",
            Default::default(),
            &Default::default(),
        )
        .await
        .unwrap();
        let text = crate::lens_text::render(&run, &Default::default());
        assert!(
            text.starts_with("(custom component `x/burndown`; its table rendering)\n"),
            "{text}"
        );
    }
}
