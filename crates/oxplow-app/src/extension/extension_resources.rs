//! An extension's `resources:` (`.context/resources.md`): what its URIs
//! may name. Every resource is declared this way — oxplow-foundation's
//! included — so none is special:
//!
//! ```yaml
//! resources:
//!   - name: pr                    # oxplow:acme/pr?id=12
//!     label: Pull request
//!     params: { id: { required: true, pattern: '^[0-9]+$' } }
//!     resolve: prs                # a dataset of its own with `ref` and `title`
//!     searchable: prs             # …and `body`, which search indexes
//!     icon: git-pull-request
//!     wikilink: { prefix: pr, param: id }   # [[pr:12]]
//! ```
//!
//! A resource is `<extension>/<name>`, where the extension segment is the
//! extension's name, or `oxplow` for the extension whose namespace is
//! oxplow's (only a bundled one may be). That one resolves to core's
//! datasets (`resolve: effort` is `v_effort`); any other to its own
//! (`v_<extension>_<dataset>`). Collisions across extensions are the
//! vocabulary's to find.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use oxplow_domain::uri::resources::{ParamDecl, ResourceDecl};

use crate::extension::extensions::manifest_v2::{at, item_lines, key_line};
use crate::extension::extensions::CORE_NAMESPACE;

/// The icons a resource, a page or an action may name (lucide names; the
/// desktop maps each).
pub const ICONS: &[&str] = &[
    "activity",
    "archive",
    "at-sign",
    "bar-chart-3",
    "bell",
    "book-open",
    "box",
    "bug",
    "calendar",
    "check-square",
    "circle-check",
    "circle-dot",
    "circle-stop",
    "columns-3",
    "copy",
    "database",
    "download",
    "external-link",
    "file-text",
    "flag",
    "folder",
    "folder-tree",
    "git-branch",
    "git-commit",
    "git-compare",
    "git-merge",
    "git-pull-request",
    "history",
    "inbox",
    "layout-dashboard",
    "link",
    "list-tree",
    "maximize-2",
    "message-square",
    "package",
    "pencil",
    "play",
    "plus",
    "refresh-cw",
    "server",
    "settings",
    "shield",
    "sparkles",
    "star",
    "tag",
    "terminal",
    "ticket",
    "trash-2",
    "triangle-alert",
    "user",
    "zap",
];

/// Where an extension keeps the svg a resource's `icon:` may name.
const SVG_ICON_DIR: &str = "assets/";

/// The largest svg a resource may bring.
const MAX_SVG_BYTES: usize = 32 * 1024;

/// Wikilink sugar oxplow-foundation declares (`dir:`,
/// `finding:`): another extension's `wikilink.prefix` may not take it. A work
/// list's own ids (`tsk42`) are its id pattern's, not a prefix core keeps.
const RESERVED_PREFIXES: &[&str] = &["dir", "finding"];

/// Why `name` can't name a resource from a manifest (a decorator's
/// `resource:`, a prompt's `about:`): not `<extension>/<name>`, or one of
/// oxplow's that oxplow-foundation doesn't declare. Another extension's
/// is the vocabulary's to know.
pub fn resource_problem(name: &str) -> Option<String> {
    let parsed: oxplow_domain::uri::ResourceName = match name.parse() {
        Ok(n) => n,
        Err(_) => {
            return Some(
                "a resource is `<extension>/<name>` (`oxplow/work_item`, `oxplow/commit`)".into(),
            )
        }
    };
    (parsed.extension() == CORE_NAMESPACE
        && oxplow_domain::uri::resources::foundation()
            .get(&parsed)
            .is_none())
    .then(|| "oxplow declares no such resource".to_string())
}

/// Why `svg` can't be drawn as an icon: not an svg, too big, or not inert
/// (script, an event handler, foreign content), which an `<img>` would not
/// run but a file shared with a team should never carry.
fn svg_problem(svg: &str) -> Option<String> {
    if svg.len() > MAX_SVG_BYTES {
        return Some(format!("an icon is at most {} KiB", MAX_SVG_BYTES / 1024));
    }
    let lower = svg.to_ascii_lowercase();
    let body = lower.trim_start();
    let body = match body.strip_prefix("<?xml") {
        Some(rest) => rest.split_once("?>").map_or("", |(_, r)| r.trim_start()),
        None => body,
    };
    if !body.starts_with("<svg") {
        return Some("the file isn't an svg (it starts with `<svg`)".into());
    }
    let handler = lower.match_indices("on").any(|(i, _)| {
        let before = lower[..i].chars().next_back();
        let after = &lower[i + 2..];
        before.is_some_and(|c| c.is_whitespace() || c == '"' || c == '\'')
            && after
                .trim_start_matches(|c: char| c.is_ascii_lowercase())
                .starts_with('=')
            && after.chars().next().is_some_and(|c| c.is_ascii_lowercase())
    });
    if lower.contains("<script")
        || lower.contains("javascript:")
        || lower.contains("<foreignobject")
        || handler
    {
        return Some(
            "an icon is inert: no script, event handlers (`onload=`) or `foreignObject`".into(),
        );
    }
    None
}

/// Read the svg each `icon: assets/<x>.svg` names, and publish it in the
/// resource as a `data:` URI (the desktop draws it as an image). One that
/// isn't there, isn't inside the extension, isn't an svg or isn't inert is
/// an error at the resource's line, and the resource is dropped.
pub(crate) fn inline_svg_icons(
    resources: Vec<ExtensionResource>,
    errors: &mut Vec<String>,
    files: &dyn crate::extension::extensions::ExtensionFiles,
) -> Vec<ExtensionResource> {
    use base64::Engine as _;
    resources
        .into_iter()
        .filter_map(|mut r| {
            let Some(path) = r.decl.icon.clone().filter(|i| i.starts_with(SVG_ICON_DIR)) else {
                return Some(r);
            };
            let why = (|| {
                if path.contains("..") || !oxplow_domain::service::valid_path(&path) {
                    return Some("the icon must be a file inside the extension".to_string());
                }
                if !path.ends_with(".svg") {
                    return Some("an icon file is an svg (`assets/<name>.svg`)".to_string());
                }
                match files.read(&path) {
                    None => Some(format!("no file `{path}` in the extension")),
                    Some(svg) => {
                        if let Some(why) = svg_problem(&svg) {
                            return Some(format!("`{path}`: {why}"));
                        }
                        r.decl.icon = Some(format!(
                            "data:image/svg+xml;base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(svg)
                        ));
                        None
                    }
                }
            })();
            match why {
                None => Some(r),
                Some(why) => {
                    errors.push(format!(
                        "{}: resource `{}`: {why}",
                        r.declared_at, r.decl.name
                    ));
                    None
                }
            }
        })
        .collect()
}

/// A resource an extension declares (valid ones; invalid ones are in its
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResource {
    pub decl: ResourceDecl,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

/// A `resources:` entry as the manifest holds it.
pub(crate) type ResourceFile = oxplow_domain::uri::resources::ResourceEntry;

/// The extension segment of the resources `extension` declares.
pub fn resource_extension(extension: &str, namespace: &str) -> String {
    if namespace == CORE_NAMESPACE {
        CORE_NAMESPACE.to_string()
    } else {
        extension.to_string()
    }
}

/// Parse a `resources:` block against the extension's datasets (core's,
/// for oxplow's namespace): the valid resources, and an error
/// (`file:line: …`) for each broken one.
pub fn parse_resources(
    extension: &str,
    namespace: &str,
    datasets: &[oxplow_db::datasets::DatasetSource],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<ExtensionResource>, Vec<String>) {
    let block = key_line(manifest, "resources");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`resources` must be a list")],
        );
    };
    let segment = resource_extension(extension, namespace);
    let core = namespace == CORE_NAMESPACE;
    let core_datasets = if core {
        oxplow_db::datasets::core_sources().unwrap_or_default()
    } else {
        Vec::new()
    };
    let datasets = if core { &core_datasets[..] } else { datasets };
    let lines = item_lines(manifest, "resources");
    let mut out: Vec<ExtensionResource> = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: ResourceFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, line, format!("resource: {e}")));
                continue;
            }
        };
        if out.iter().any(|r| r.decl.name.name() == f.name) {
            errors.push(at(
                file,
                line,
                format!("resource `{}` is declared twice", f.name),
            ));
            continue;
        }
        let declared_at = at(file, line, "").trim_end_matches(": ").to_string();
        match decl_of(&segment, extension, core, datasets, f) {
            Ok(decl) => out.push(ExtensionResource { decl, declared_at }),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn decl_of(
    segment: &str,
    extension: &str,
    core: bool,
    datasets: &[oxplow_db::datasets::DatasetSource],
    f: ResourceFile,
) -> Result<ResourceDecl, String> {
    let declared = f.clone().into_decl(segment)?;
    let named = |m: String| format!("resource `{}`: {m}", f.name);
    let mut params = BTreeMap::new();
    for (key, p) in f.params {
        let mut k = key.chars();
        let is_key = k.next().is_some_and(|c| c.is_ascii_alphabetic())
            && k.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !is_key {
            return Err(named(format!(
                "param `{key}`: a key is a letter, then letters, digits, `_`, `.` or `-`"
            )));
        }
        let pattern = match p.pattern {
            Some(pattern) => Some(
                crate::param_pattern::param_pattern(&pattern)
                    .map_err(|why| named(format!("param `{key}`'s `pattern: {pattern}`: {why}")))?,
            ),
            None => None,
        };
        params.insert(
            key,
            ParamDecl {
                required: p.required,
                pattern,
            },
        );
    }
    // A dataset with these columns: the extension's own, or core's.
    let view_with = |key: &str, dataset: &str, columns: &[&str], why: &str| {
        let source = datasets
            .iter()
            .find(|m| m.decl.name == dataset)
            .ok_or_else(|| {
                named(format!(
                    "`{key}: {dataset}` isn't one of {} datasets",
                    if core {
                        "core's"
                    } else {
                        "this extension's (`datasets:`)"
                    }
                ))
            })?;
        if let Some(col) = columns
            .iter()
            .find(|col| !source.decl.fields.iter().any(|c| c.name == **col))
        {
            return Err(named(format!(
                "dataset `{dataset}` has no `{col}` column ({why})"
            )));
        }
        Ok(if core {
            oxplow_db::datasets::core_view(dataset)
        } else {
            oxplow_db::datasets::extension_view(extension, dataset)
        })
    };
    let title = f.title.as_deref().unwrap_or("title");
    let resolve = f
        .resolve
        .as_deref()
        .map(|m| view_with("resolve", m, &["uri", title], "it gives each one's title"))
        .transpose()?;
    let searchable = f
        .searchable
        .as_deref()
        .map(|m| {
            view_with(
                "searchable",
                m,
                &["uri", "title", "body"],
                "what search indexes",
            )
        })
        .transpose()?;
    if let Some(w) = &f.wikilink {
        if !params.contains_key(&w.param) {
            return Err(named(format!(
                "`wikilink.param: {}` isn't one of its params",
                w.param
            )));
        }
        let mut p = w.prefix.chars();
        let is_prefix = p.next().is_some_and(|c| c.is_ascii_lowercase())
            && p.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !is_prefix {
            return Err(named(format!(
                "`wikilink.prefix: {}` must be lowercase letters, digits and `_`",
                w.prefix
            )));
        }
        if !core && RESERVED_PREFIXES.contains(&w.prefix.as_str()) {
            return Err(named(format!(
                "`wikilink.prefix: {}` is oxplow's own sugar (`[[{}:…]]`); choose another",
                w.prefix, w.prefix
            )));
        }
    }
    if let Some(icon) = &f.icon {
        // An `assets/<x>.svg` is the extension's own, read when it loads
        // ([`inline_svg_icons`]); any other is a name oxplow draws.
        if !icon.starts_with(SVG_ICON_DIR) && !ICONS.contains(&icon.as_str()) {
            return Err(named(format!(
                "`icon: {icon}` isn't one oxplow draws; use one of {} (or `assets/<name>.svg`, an svg in the extension)",
                ICONS.join(", ")
            )));
        }
    }
    Ok(ResourceDecl {
        params,
        resolve,
        searchable,
        ..declared
    })
}

#[cfg(test)]
mod tests {
    use crate::extension::extensions::load_extensions;

    const MANIFEST: &str = "manifest: 2
name: acme
intent: { purpose: PRs., origin: null, examples: [] }
datasets:
  - name: prs
    version: 1
    description: Pull requests.
    fields:
      - { name: uri, type: TEXT, doc: The ref. }
      - { name: title, type: TEXT, doc: Its title. }
resources:
  - name: pr
    label: Pull request
    params: { id: { required: true, pattern: '^\\d+$' } }
    resolve: prs
    icon: git-pull-request
    wikilink: { prefix: pr, param: id }
";

    fn acme(manifest: &str) -> crate::extension::extensions::Extension {
        acme_with(manifest, &[])
    }

    /// `acme` with more files written in its folder first.
    fn acme_with(
        manifest: &str,
        files: &[(&str, &str)],
    ) -> crate::extension::extensions::Extension {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".oxplow/extensions/acme");
        std::fs::create_dir_all(root.join("datasets")).unwrap();
        std::fs::write(root.join("extension.yaml"), manifest).unwrap();
        for (rel, text) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::fs::write(
            root.join("datasets/prs.sql"),
            "SELECT CAST('oxplow:acme/pr?id=1' AS TEXT) AS uri, CAST('One' AS TEXT) AS title",
        )
        .unwrap();
        load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap()
    }

    #[test]
    fn a_resource_loads_under_its_extension_with_its_dataset() {
        let ext = acme(MANIFEST);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let r = &ext.resources[0];
        assert_eq!(r.decl.name.to_string(), "acme/pr");
        assert_eq!(r.decl.resolve.as_deref(), Some("v_acme_prs"));
        // Its pattern is spelled out the way both engines read alike.
        assert_eq!(r.decl.params["id"].pattern.as_deref(), Some("^[0-9]+$"));
        assert!(r.decl.params["id"].required);
        assert_eq!(r.declared_at, ".oxplow/extensions/acme/extension.yaml:12");
    }

    #[test]
    fn a_broken_resource_is_an_error_at_its_line() {
        for (from, to, says) in [
            ("name: pr\n", "name: Pr\n", "a name is lowercase"),
            ("pattern: '^\\d+$'", "pattern: '\\d+'", "anchored"),
            (
                "resolve: prs",
                "resolve: gone",
                "isn't one of this extension's",
            ),
            ("param: id }", "param: number }", "isn't one of its params"),
            ("prefix: pr,", "prefix: Pr,", "lowercase letters"),
            ("prefix: pr,", "prefix: dir,", "oxplow's own sugar"),
            (
                "icon: git-pull-request",
                "icon: rocket",
                "isn't one oxplow draws",
            ),
            ("params: { id:", "params: { 'bad key':", "a key is a letter"),
        ] {
            let ext = acme(&MANIFEST.replace(from, to));
            let errors = ext.errors.join("\n");
            assert!(ext.resources.is_empty(), "{to}: {:?}", ext.resources);
            assert!(
                errors.contains(says) && errors.contains("extension.yaml:12"),
                "{to}: {errors}"
            );
        }
    }

    const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><circle cx=\"12\" cy=\"12\" r=\"9\"/></svg>";

    /// A resource may name its own svg in the extension's folder; it is read
    /// at load and published with the resource as a data URI, so the
    /// desktop draws it without a request.
    #[test]
    fn a_resource_may_bring_its_own_svg_icon() {
        let ext = acme_with(
            &MANIFEST.replace("icon: git-pull-request", "icon: assets/pr.svg"),
            &[("assets/pr.svg", SVG)],
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let icon = ext.resources[0].decl.icon.clone().unwrap();
        let encoded = icon
            .strip_prefix("data:image/svg+xml;base64,")
            .unwrap_or_else(|| panic!("{icon}"));
        let svg =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).unwrap();
        assert_eq!(String::from_utf8(svg).unwrap(), SVG);
    }

    /// The file must be there, inside the extension, an svg, and inert.
    #[test]
    fn a_resource_icon_file_is_checked_when_the_extension_loads() {
        let icon =
            |path: &str| MANIFEST.replace("icon: git-pull-request", &format!("icon: {path}"));
        for (manifest, files, says) in [
            (icon("assets/gone.svg"), vec![], "assets/gone.svg"),
            (
                icon("assets/../extension.yaml"),
                vec![],
                "inside the extension",
            ),
            (
                icon("assets/pr.png"),
                vec![("assets/pr.png", SVG)],
                "an svg",
            ),
            (
                icon("assets/pr.svg"),
                vec![("assets/pr.svg", "<html></html>")],
                "isn't an svg",
            ),
            (
                icon("assets/pr.svg"),
                vec![("assets/pr.svg", "<svg><script>alert(1)</script></svg>")],
                "script",
            ),
            (
                icon("assets/pr.svg"),
                vec![("assets/pr.svg", "<svg onload=\"x()\"></svg>")],
                "script",
            ),
        ] {
            let ext = acme_with(&manifest, &files);
            let errors = ext.errors.join("\n");
            assert!(ext.resources.is_empty(), "{says}: {:?}", ext.resources);
            assert!(errors.contains(says), "{says}: {errors}");
        }
    }

    /// The icons oxplow draws itself only shrink: an extension brings its own
    /// svg instead of a new name here. A name missing from the golden is a
    /// new built-in icon; a golden name missing from the list is one that
    /// left — take it out of the golden too.
    #[test]
    fn the_core_icons_only_shrink() {
        let golden: Vec<String> =
            serde_json::from_str(include_str!("../../fixtures/core_icons.json")).unwrap();
        let added: Vec<&&str> = super::ICONS
            .iter()
            .filter(|i| !golden.iter().any(|g| g == **i))
            .collect();
        assert!(
            added.is_empty(),
            "new core icons {added:?}: bring an svg instead"
        );
        let left: Vec<&String> = golden
            .iter()
            .filter(|g| !super::ICONS.contains(&g.as_str()))
            .collect();
        assert!(
            left.is_empty(),
            "left the list: {left:?} — remove it from the golden"
        );
    }

    #[test]
    fn a_dataset_that_titles_nothing_is_refused() {
        let ext =
            acme(&MANIFEST.replace("      - { name: title, type: TEXT, doc: Its title. }\n", ""));
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains("dataset `prs` has no `title` column"),
            "{errors}"
        );
    }

    // oxplow's own resources are declared the same way, by
    // oxplow-foundation, and resolve to core's datasets.
    #[test]
    fn oxplows_resources_are_foundations_and_resolve_to_core_datasets() {
        let dir = tempfile::tempdir().unwrap();
        let foundation = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-foundation")
            .expect("foundation ships");
        assert!(foundation.errors.is_empty(), "{:?}", foundation.errors);
        let effort = foundation
            .resources
            .iter()
            .find(|r| r.decl.name.to_string() == "oxplow/effort")
            .expect("foundation declares oxplow/effort");
        assert_eq!(effort.decl.resolve.as_deref(), Some("v_effort"));
    }

    /// The desktop draws exactly the icons an extension may name: its
    /// `ICONS` map (`apps/desktop/src/icons.ts`) has a key for
    /// each, and no other.
    #[test]
    fn the_desktop_draws_every_allowed_icon() {
        let ts = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src/icons.ts"),
        )
        .unwrap();
        let start = ts.find("ICONS").unwrap();
        let body = &ts[start..start + ts[start..].find("};").unwrap()];
        let mut drawn: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
            .collect();
        drawn.sort();
        let mut allowed = super::ICONS.to_vec();
        allowed.sort();
        assert_eq!(drawn, allowed);
    }
}
