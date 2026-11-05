//! `custom_components:`: a web bundle a page's `component` node (or a
//! `viz: custom` node) renders in a sandboxed frame, fed the page's loader
//! results. The frame has no origin, no network and no daemon token, so it
//! reaches only what it declares: its own read-only SQL when it says `sql:
//! read`, and the commands it may invoke (`commands`). It renders without
//! approval;
//! one that declares `commands` is a program a person approves before its
//! `invoke` runs them with their rights (tsk960), at the version of the
//! bundle it loaded (tsk984). A bundled extension's component runs the same
//! way: its bundle is served from the embedded files, hashed and approved
//! like a project's.
//!
//! ```yaml
//! custom_components:
//!   - id: burndown
//!     title: Burndown
//!     bundle: components/burndown        # default components/<id>; holds index.html
//!     commands: [oxplow.work_item.transition]
//! ```

use oxplow_domain::diagnostic::Diagnostic;
use std::path::Path;

use oxplow_domain::extension_path::is_extension_relative;
use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, item_lines, key_line};

/// A declared component (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomComponent {
    /// `[a-z0-9-]+`, unique in the extension; a page's `component` node
    /// names it.
    pub id: String,
    pub extension: String,
    pub title: Option<String>,
    /// The bundle's folder inside the extension, holding `index.html`.
    pub bundle: String,
    /// Commands the component may invoke.
    pub commands: Vec<String>,
    /// It runs its own read-only SQL over the datasets (`querySql`): needs
    /// `sql.read` permission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<oxplow_domain::permissions::SqlAccess>,
    /// One sentence saying what it is (`v_custom_component.description`);
    /// `None` when its declaration says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A `custom_components:` entry as the manifest holds it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComponentFile {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    bundle: Option<String>,
    #[serde(default)]
    commands: Vec<String>,
    /// `read`: it runs its own read-only SQL over the datasets
    /// (`querySql`); its extension needs `sql.read` in `permissions:`.
    #[serde(default)]
    sql: Option<oxplow_domain::permissions::SqlAccess>,
    /// One sentence saying what the component is.
    #[serde(default)]
    description: Option<String>,
}

/// What a bundle folder holds, as far as loading cares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleStat {
    pub has_index: bool,
    pub files: usize,
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

/// What files under `rel` say about a bundle, for a tree of paths (a
/// revision): their count and an `index.html`; `Unknown` when none are
/// there.
pub fn look_in_paths<'a>(rel: &str, files: impl Iterator<Item = &'a str>) -> BundleLook {
    let prefix = format!("{}/", rel.trim_end_matches('/'));
    let mut stat = BundleStat::default();
    for path in files {
        let Some(inner) = path.strip_prefix(&prefix) else {
            continue;
        };
        stat.files += 1;
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
/// under review, a revision, the folder on disk, or a bundled extension's
/// embedded files). A page `read` can't find (a revision's tree rarely holds a built bundle)
/// is skipped: loading says when a bundle has no `index.html`.
pub fn bundle_problems(
    ext: &super::Extension,
    read: &dyn Fn(&str) -> Option<String>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for c in &ext.custom_components {
        let rel = format!("{}/index.html", c.bundle.trim_end_matches('/'));
        let Some(html) = read(&rel) else {
            continue;
        };
        out.extend(page_problems(&html, true).into_iter().map(|p| {
            p.at(&rel, None)
                .contribution(format!("{}/{}", ext.name, c.id))
        }));
    }
    out
}

/// What `ext`'s components do that works but goes against how a page is
/// built, as check warnings at the file: a script its `index.html` loads
/// from the bundle that keeps state in browser storage. A component's view
/// is its page's view params (`component.setViewParams`), kept in its tab, so Back,
/// a bookmark and a reopened tab keep it; storage keeps it nowhere oxplow
/// knows.
pub fn bundle_warnings(
    ext: &super::Extension,
    read: &dyn Fn(&str) -> Option<String>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for c in &ext.custom_components {
        let bundle = c.bundle.trim_end_matches('/');
        let Some(html) = read(&format!("{bundle}/index.html")) else {
            continue;
        };
        for tag in tags(&html).into_iter().filter(|t| t.name == "script") {
            let Some(src) = tag
                .attrs
                .iter()
                .find(|(k, _)| k == "src")
                .map(|(_, v)| v.trim())
            else {
                continue;
            };
            // Only the bundle's own files: oxplow's library and anything
            // outside are not the component's code.
            if src.starts_with('/') || src.contains("://") || src.starts_with("data:") {
                continue;
            }
            let rel = format!("{bundle}/{}", src.trim_start_matches("./"));
            let Some(script) = read(&rel) else { continue };
            for store in ["localStorage", "sessionStorage"] {
                if script.contains(store) {
                    out.push(
                        Diagnostic::warning(
                            "component.browser_storage",
                            format!(
                                "it uses `{store}` — a component keeps its view in its page's \
                                 view params, which its tab keeps (`component.setViewParams`, \
                                 `onViewParams`), this step's in its history state (`setHistoryState`) and \
                                 its own memory for the person in `component.pageMemory`"
                            ),
                        )
                        .at(&rel, None)
                        .contribution(format!("{}/{}", ext.name, c.id)),
                    );
                }
            }
        }
    }
    out
}

/// What one of a bundle's pages asks for that the bundle's CSP refuses
/// silently or that ends the component: an inline `<script>` or event
/// handler, a `type="module"` script, a load from outside the bundle
/// (`src`, `srcset`, a stylesheet), a `<base>`, a nested frame, an `<object>` or `<embed>`,
/// a refresh; and, for its `index.html`, never loading the client
/// library. A link (`<a href>`) is a navigation, not a load, and isn't one.
/// A `<form>` is fine: the frame allows forms (`allow-forms`) and its CSP
/// never lets one submit anywhere (`form-action 'none'`), so a component
/// handles `submit` in its script and runs a command with `invoke`.
pub fn page_problems(html: &str, index: bool) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut loads_lib = false;
    let outside_bundle = |url: &str| {
        Diagnostic::error(
            "component.outside_bundle",
            format!(
                "`{url}` is outside the bundle — the frame's CSP loads only the bundle's own \
                 files and oxplow's `/component-lib/`"
            ),
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
                    out.push(
                        Diagnostic::error(
                            "component.inline_event_handler",
                            format!(
                                "an inline event handler (`{name}`) — the frame's CSP refuses it"
                            ),
                        )
                        .fix("add the listener from a script file"),
                    );
                }
            }
        }
        let refused = match tag.name.as_str() {
            "base" => Some("a <base> — the frame's CSP refuses one (`base-uri 'none'`)"),
            "iframe" | "frame" => Some("a <iframe> — the frame's CSP refuses a nested frame"),
            "object" => Some("an <object> — the frame's CSP refuses embedded objects"),
            "embed" => Some("an <embed> — the frame's CSP refuses embedded objects"),
            "meta"
                if attr("http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh")) =>
            {
                Some("a refresh — it navigates the frame away, which ends the component")
            }
            _ => None,
        };
        out.extend(refused.map(|m| Diagnostic::error("component.refused_markup", m)));
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
                    Diagnostic::error(
                        "component.module_script",
                        "a `type=\"module\"` script — a sandboxed frame can't load modules",
                    )
                    .fix("use a classic script"),
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
                    Diagnostic::error(
                        "component.inline_script",
                        "an inline <script> — the frame's CSP runs only script files",
                    )
                    .fix("move it into a `.js` file"),
                );
            }
        }
    }
    if index && !loads_lib {
        out.push(
            Diagnostic::error(
                "component.no_client_library",
                "doesn't load the client library, which is how a component talks to oxplow",
            )
            .fix(format!("add `<script src=\"{LIB_SCRIPT}\"></script>`")),
        );
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
    oxplow_domain::contribution::valid_local(
        oxplow_domain::contribution::ContributionKind::CustomComponent,
        id,
    )
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
) -> (Vec<CustomComponent>, Vec<Diagnostic>) {
    let block = key_line(manifest, "custom_components");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "component.not_a_list",
                file,
                block,
                "`custom_components` must be a list",
            )],
        );
    };
    let mut out: Vec<CustomComponent> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "custom_components");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let c: ComponentFile = match serde_yaml::from_value(item.clone()) {
            Ok(c) => c,
            Err(e) => {
                errors.push(at(
                    "component.invalid",
                    file,
                    item_line,
                    format!("custom component: {e}"),
                ));
                continue;
            }
        };
        let line = item_line;
        let bundle = c
            .bundle
            .clone()
            .unwrap_or_else(|| format!("components/{}", c.id));
        let problem = if !valid_id(&c.id) {
            Some((
                "component.invalid_id",
                format!(
                    "custom component id `{}` must be lowercase letters, digits and dashes",
                    c.id
                ),
            ))
        } else if out.iter().any(|o| o.id == c.id) {
            Some((
                "component.duplicate",
                format!("custom component `{}` is declared twice", c.id),
            ))
        } else if !is_extension_relative(&bundle) {
            Some((
                "component.bundle_outside",
                format!(
                    "custom component `{}`: bundle `{bundle}` must be a folder inside the extension",
                    c.id
                ),
            ))
        } else if let Some(bad) = c
            .commands
            .iter()
            .find(|n| oxplow_domain::CommandSpec::validate_id(n).is_err())
        {
            Some((
                "component.invalid_command_name",
                format!("custom component `{}`: `{bad}` isn't a command name", c.id),
            ))
        } else {
            match stat(&bundle) {
                BundleLook::Unknown => None,
                BundleLook::Absent => Some((
                    "component.bundle_missing",
                    format!(
                        "custom component `{}`: bundle `{bundle}` isn't a folder in the extension",
                        c.id
                    ),
                )),
                BundleLook::Found(s) if s.symlink.is_some() => Some((
                    "component.bundle_symlink",
                    format!(
                        "custom component `{}`: bundle `{bundle}` holds a symlink (`{}`); a bundle is \
                         plain files",
                        c.id,
                        s.symlink.unwrap_or_default()
                    ),
                )),
                BundleLook::Found(s) if !s.has_index => Some((
                    "component.bundle_no_index",
                    format!(
                        "custom component `{}`: bundle `{bundle}` has no `index.html`",
                        c.id
                    ),
                )),
                BundleLook::Found(_) => None,
            }
        };
        match problem {
            Some((code, p)) => errors.push(at(code, file, line, p)),
            None => out.push(CustomComponent {
                id: c.id,
                extension: extension.to_string(),
                title: c.title,
                bundle,
                commands: c.commands,
                sql: c.sql,
                description: c
                    .description
                    .map(|d| d.trim().to_string())
                    .filter(|d| !d.is_empty()),
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
        let manifest = "manifest: 2\nname: acme\nintent: { purpose: p, origin: null, examples: [] }\ncustom_components:\n  - { id: chart, bundle: ui/chart }\n";
        let tree = |extra: &[(&str, &str)]| {
            crate::extension::extensions::Tree::new(
                std::iter::once(("extension.yaml".to_string(), manifest.to_string()))
                    .chain(extra.iter().map(|(p, b)| (p.to_string(), b.to_string()))),
            )
            .load("acme", ".oxplow/extensions/acme")
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
            said(&broken.errors).contains("has no `index.html`"),
            "{:?}",
            broken.errors
        );
    }

    use super::{bundle_problems, bundle_warnings, page_problems, Diagnostic};

    fn said(d: &[Diagnostic]) -> String {
        d.iter()
            .map(|d| d.render(""))
            .collect::<Vec<_>>()
            .join("\n")
    }
    use crate::extension::extensions::{load_extensions, Extension};
    use crate::pages::schema::NodeKind;
    use crate::pages::spec::{Viz, VizConfig};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const PAGE: &str = "title: Burn\nloaders:\n  burn: { sql: \"SELECT 1 AS day, 3 AS remaining\" }\n\
                        view:\n  from: burn\n  viz: custom\n  custom: { component: burndown, props: { color: accent } }\n";

    /// The config of the first custom viz node of `ext`'s page `slug`.
    fn custom_viz<'e>(ext: &'e Extension, slug: &str) -> &'e VizConfig {
        let page = ext.page_files.iter().find(|p| p.slug == slug).unwrap();
        page.nodes()
            .into_iter()
            .find_map(|n| match &n.kind {
                NodeKind::Viz { config, .. } if config.viz == Viz::Custom => Some(config),
                _ => None,
            })
            .expect("a custom viz")
    }

    /// Extension `x` declaring `components` (the block's
    /// entries), with a `burndown` bundle and the `burn` page.
    fn load(root: &Path, components: &str) -> Extension {
        load_permitted(root, "", components)
    }

    /// [`load`] with a `permissions:` block (one line, or none).
    fn load_permitted(root: &Path, permissions: &str, components: &str) -> Extension {
        write(
            root,
            ".oxplow/extensions/x/extension.yaml",
            &format!(
                "manifest: 2\nname: x\nengine: \">=0.1\"\nversion: 0.1.0\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\n{permissions}custom_components:\n{components}"
            ),
        );
        write(
            root,
            ".oxplow/extensions/x/components/burndown/index.html",
            &format!("<!doctype html>{LIB_TAG}<script src=\"app.js\"></script>"),
        );
        write(root, ".oxplow/extensions/x/components/burndown/app.js", "1");
        write(root, ".oxplow/extensions/x/pages/burn.yaml", PAGE);
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
        assert!(page_problems(&ok, true).is_empty());
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
            let problems = said(&page_problems(&format!("{LIB_TAG}{html}"), true));
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
            let problems = said(&page_problems(&format!("{LIB_TAG}{html}"), true));
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
                Vec::<Diagnostic>::new(),
                "{inside}"
            );
        }
        // tsk994: no panic on an `=` attribute before a multibyte char, and
        // it doesn't swallow the next tag.
        assert!(page_problems(&format!("{LIB_TAG}<a =\"x\"é>"), true).is_empty());
        assert!(said(&page_problems(
            &format!("{LIB_TAG}<a =\"x\"><script>evil()</script>"),
            true
        ))
        .contains("an inline <script>"));
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
                Vec::<Diagnostic>::new(),
                "{fine}"
            );
        }
        assert_eq!(
            page_problems(
                "<script src=\"/component-lib/oxplow-component.js?v=2\"></script>",
                true
            ),
            Vec::<Diagnostic>::new(),
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
            (
                "<meta http-equiv=\"refresh\" content=\"0;url=x\">",
                "a refresh",
            ),
        ] {
            let problems = said(&page_problems(&format!("{LIB_TAG}{html}"), true));
            assert!(problems.contains(says), "{html}: {problems}");
        }
        assert_eq!(
            page_problems(
                &format!(
                    "{LIB_TAG}<form><input name=title><button type=submit>Save</button></form>"
                ),
                true
            ),
            Vec::<Diagnostic>::new(),
            "a form is a component's own, handled in its script"
        );
        assert!(said(&page_problems("<script src=app.js></script>", true))
            .contains("doesn't load the client library"));
        assert_eq!(
            page_problems("<p>another page</p>", false),
            Vec::<Diagnostic>::new(),
            "only index.html must load it"
        );
    }

    /// tsk994: the lints read the files of the extension being checked —
    /// a candidate under review is not what's installed — through the read
    /// the check is given.
    #[test]
    fn a_bundles_page_is_read_from_the_files_being_checked() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "  - { id: burndown }\n");
        let candidate = |rel: &str| {
            (rel == "components/burndown/index.html")
                .then(|| "<!doctype html><script>render()</script>".to_string())
        };
        let problems = said(&bundle_problems(&ext, &candidate));
        assert!(
            problems.contains("components/burndown/index.html: an inline <script>"),
            "{problems}"
        );
        // A version without the built page (a revision's tree) has nothing
        // to lint: loading says when a bundle has no `index.html`.
        let missing = |_: &str| None;
        assert_eq!(bundle_problems(&ext, &missing), Vec::<Diagnostic>::new());
    }

    /// A component keeps its view in its page's view params, never
    /// in browser storage: a script its page loads that reads or writes
    /// `localStorage` / `sessionStorage` is a warning at that script; the
    /// client library and a script without it are not.
    #[test]
    fn a_bundle_that_keeps_state_in_browser_storage_is_warned() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "  - { id: burndown }\n");
        let files = |rel: &str| match rel {
            "components/burndown/index.html" => Some(
                r#"<!doctype html><script src="/component-lib/oxplow-component.js"></script>
                   <script src="app.js"></script><script src="./lib/plain.js"></script>"#
                    .to_string(),
            ),
            "components/burndown/app.js" => {
                Some("const last = localStorage.getItem('range');".to_string())
            }
            "components/burndown/lib/plain.js" => Some("export const x = 1;".to_string()),
            _ => None,
        };
        let warnings = bundle_warnings(&ext, &files);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0]
                .render("")
                .starts_with("components/burndown/app.js: ")
                && warnings[0].message.contains("`localStorage`")
                && warnings[0].message.contains("setViewParams"),
            "{warnings:?}"
        );
    }

    /// tsk961: the check reports a bundle's page problems at the page —
    /// a bundled extension's as a project's: they share one runtime.
    #[tokio::test]
    async fn a_bundles_pages_are_checked_whatever_its_origin() {
        let d = tempfile::tempdir().unwrap();
        load(d.path(), "  - { id: burndown }\n");
        write(
            d.path(),
            ".oxplow/extensions/x/components/burndown/index.html",
            "<!doctype html><script>render()</script>",
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension::extension_catalog::ExtensionCatalog::new();
        let v = crate::extension::extensions::validate_extension(&layer, &cat, d.path(), "x", None)
            .await
            .unwrap();
        let errs = said(&v.errors);
        assert!(
            errs.contains("components/burndown/index.html: an inline <script>"),
            "{errs}"
        );
        assert!(errs.contains("doesn't load the client library"), "{errs}");
        let mut ext = load(d.path(), "  - { id: burndown }\n");
        ext.origin = "bundled".into();
        let page = |rel: &str| {
            (rel == "components/burndown/index.html")
                .then(|| "<!doctype html><script>render()</script>".to_string())
        };
        let problems = said(&bundle_problems(&ext, &page));
        assert!(problems.contains("an inline <script>"), "{problems}");
        assert!(!problems.contains("comes with oxplow"), "{problems}");
    }

    /// A bundle has no size limit: it is served from its deployment on
    /// disk, never held in memory (a deploy warns when an extension is big).
    #[test]
    fn a_bundle_of_any_size_loads() {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            ".oxplow/extensions/x/components/burndown/index.html",
            "x",
        );
        for i in 0..300 {
            write(
                d.path(),
                &format!(".oxplow/extensions/x/components/burndown/f{i}.js"),
                "x",
            );
        }
        let ext = load(d.path(), "  - { id: burndown }\n");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.custom_components.len(), 1);
    }

    /// A component's `description:` is kept, trimmed; one without it has none
    /// (nothing is made up).
    #[test]
    fn a_component_keeps_its_description_and_never_invents_one() {
        let d = tempfile::tempdir().unwrap();
        let ext = load_permitted(
            d.path(),
            "permissions: [commands:oxplow.work_item.transition]\n",
            "  - { id: burndown, title: Burndown, description: '  Work left in a sprint.  ', commands: [oxplow.work_item.transition] }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(
            ext.custom_components[0].description.as_deref(),
            Some("Work left in a sprint.")
        );
        let d = tempfile::tempdir().unwrap();
        let plain = load_permitted(
            d.path(),
            "permissions: [commands:oxplow.work_item.transition]\n",
            "  - { id: burndown, title: Burndown, commands: [oxplow.work_item.transition] }\n",
        );
        assert_eq!(plain.custom_components[0].description, None);
    }

    #[test]
    fn a_private_extension_loads_a_component_and_its_page() {
        let d = tempfile::tempdir().unwrap();
        let ext = load_permitted(
            d.path(),
            "permissions: [commands:oxplow.work_item.transition]\n",
            "  - { id: burndown, title: Burndown, commands: [oxplow.work_item.transition] }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let c = &ext.custom_components[0];
        assert_eq!(c.bundle, "components/burndown");
        assert_eq!(c.commands, vec!["oxplow.work_item.transition"]);
        let viz = custom_viz(&ext, "burn");
        assert_eq!(
            viz.custom.as_ref().unwrap().props.as_ref().unwrap()["color"],
            "accent"
        );
    }

    #[cfg(unix)]
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
            (
                "{ id: c, bundle: components/burndown, assets: [nope] }",
                "",
                "unknown field `assets`",
            ),
            (
                "{ id: c, bundle: components/burndown, commands: [Nope] }",
                "",
                "isn't a command name",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            let base = d.path().join(".oxplow/extensions/x");
            match setup {
                "empty" => write(d.path(), ".oxplow/extensions/x/empty/readme.txt", "x"),
                "symlink" => {
                    write(d.path(), ".oxplow/extensions/x/linked/index.html", "x");
                    std::os::unix::fs::symlink("/etc/hosts", base.join("linked/hosts")).unwrap();
                }
                "linked-folder" => {
                    write(d.path(), ".oxplow/extensions/x/real/index.html", "x");
                    std::os::unix::fs::symlink(base.join("real"), base.join("aliased")).unwrap();
                }
                "index-dir" => write(
                    d.path(),
                    ".oxplow/extensions/x/dirindex/index.html/a.js",
                    "x",
                ),
                _ => {}
            }
            let ext = load(d.path(), &format!("  - {entry}\n  - {{ id: burndown }}\n"));
            let errs = said(&ext.errors);
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
    /// declares one like a private one, and its page's custom viz loads.
    #[test]
    fn a_shared_extension_loads_a_component_and_its_page() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "  - { id: burndown }\n");
        assert_eq!(ext.errors, Vec::<Diagnostic>::new());
        assert_eq!(
            ext.custom_components
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["burndown"]
        );
        assert_eq!(custom_viz(&ext, "burn").viz, Viz::Custom);
    }

    /// A page's custom viz names one of its extension's components: none,
    /// or one it doesn't declare, is an error at the page.
    #[test]
    fn a_custom_viz_needs_one_of_its_extensions_components() {
        for (page, says) in [
            (
                "title: B\nloaders:\n  b: { sql: SELECT 1 }\nview: { from: b, viz: custom }\n",
                "needs `custom: { component: <id> }`",
            ),
            (
                "title: B\nloaders:\n  b: { sql: SELECT 1 }\nview: { from: b, viz: custom, custom: { component: nope } }\n",
                "isn't one of this extension's `custom_components`",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            write(d.path(), ".oxplow/extensions/x/pages/other.yaml", page);
            let ext = load(d.path(), "  - { id: burndown }\n");
            assert!(
                said(&ext.errors).contains(says),
                "{page}: {:?}",
                ext.errors
            );
            assert!(ext.page_files.iter().all(|p| p.slug != "other"));
        }
    }

    #[tokio::test]
    async fn validation_checks_its_commands_reads_as_a_table_and_nudges_a_lookalike() {
        let d = tempfile::tempdir().unwrap();
        load_permitted(
            d.path(),
            "permissions: [commands:nope.no.cmd, commands:oxplow.work_item.transition]\n",
            "  - { id: burndown, commands: [nope.no.cmd, oxplow.work_item.transition] }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/x/pages/burn.yaml",
            "title: Burn\nloaders:\n  burn: { sql: \"SELECT 1 AS day, 3 AS remaining\" }\n\
             view:\n  from: burn\n  viz: custom\n  custom: { component: burndown }\n  chart: { x: day, y: remaining }\n",
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension::extension_catalog::ExtensionCatalog::new();
        let schema = |n: &str| {
            (n == "oxplow.work_item.transition").then(|| serde_json::json!({ "type": "object" }))
        };
        let v = crate::extension::extensions::validate_extension(
            &layer,
            &cat,
            d.path(),
            "x",
            Some(&schema),
        )
        .await
        .unwrap();
        let errs = said(&v.errors);
        assert!(
            errs.contains("declares command `nope.no.cmd`, which isn't registered"),
            "{errs}"
        );
        assert!(!errs.contains("oxplow.work_item.transition"), "{errs}");
        assert!(
            said(&v.warnings).contains("the kit's `chart` viz may already cover it"),
            "{:?}",
            v.warnings
        );
        // An agent reads a custom viz as its rows' table.
        let ext = load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap();
        let run = crate::pages::spec::VizRun {
            config: custom_viz(&ext, "burn").clone(),
            params: Default::default(),
            result: oxplow_db::SqlQueryResult {
                columns: vec!["day".into(), "remaining".into()],
                rows: vec![vec![oxplow_db::SqlCell::Int(1), oxplow_db::SqlCell::Int(3)]],
                ..Default::default()
            },
        };
        let text = crate::viz_text::render(&run, &Default::default());
        assert!(
            text.starts_with("(custom component `x/burndown`; its table rendering)\n"),
            "{text}"
        );
    }
}
