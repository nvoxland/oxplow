//! An extension's ref kinds (`ref_kinds:` in its manifest, experimental —
//! a private extension only; P8.D6): new kinds of thing a ref can name,
//! registered into the running vocabulary's `KindRegistry` by
//! `vocabulary_reactor`, so `[[acme_pr:12]]` (and `[[pr:12]]`, with a
//! `wikilink:` prefix) links while the extension is installed.
//!
//! ```yaml
//! ref_kinds:
//!   - kind: acme_pr             # <namespace>_<name>; the namespace is the extension's name, `-` → `_`
//!     label: Pull request
//!     id: '^\d+$'               # anchored
//!     resolve: prs              # one of its models, with `ref` and `title` columns
//!     page: pr                  # one of its pages, opened with `?ref=<ref>`
//!     wikilink: pr              # optional `[[pr:12]]` sugar
//!     icon: git-pull-request    # one of REF_KIND_ICONS
//! ```

use serde::{Deserialize, Serialize};

use crate::extensions::manifest_v2::{at, entry_line, key_line};
use crate::extensions::ExtensionPage;

/// The icons a ref kind may name (lucide names; the desktop maps each).
pub const REF_KIND_ICONS: &[&str] = &[
    "book-open",
    "box",
    "bug",
    "calendar",
    "circle-dot",
    "database",
    "file-text",
    "flag",
    "folder",
    "git-branch",
    "git-commit",
    "git-pull-request",
    "link",
    "message-square",
    "package",
    "server",
    "shield",
    "star",
    "tag",
    "ticket",
    "user",
    "zap",
];

/// Wikilink sugar core reads before kinds (`dir:`, `finding:`, `tsk42`):
/// a prefix may not shadow it.
const RESERVED_PREFIXES: &[&str] = &["dir", "finding", "tsk"];

/// A ref kind an extension declares (valid ones; invalid ones are in its
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RefKindDecl {
    pub kind: String,
    pub extension: String,
    pub label: String,
    /// The anchored regex its ids match.
    pub id_pattern: String,
    /// The view whose `title` names one (`v_<extension>_<model>`, by `ref`).
    pub resolve: String,
    /// The page that opens one: `page:ext.<extension>.<page>`, given
    /// `?ref=<ref>`.
    pub page: String,
    pub wikilink: Option<String>,
    pub icon: String,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefKindFile {
    kind: String,
    label: String,
    id: String,
    resolve: String,
    page: String,
    #[serde(default)]
    wikilink: Option<String>,
    icon: String,
}

/// Parse a `ref_kinds:` block against the extension's models and pages:
/// the valid kinds, and an error (`file:line: …`) for each broken one.
/// Collisions with core and other extensions are the reactor's to find.
pub fn parse_ref_kinds(
    extension: &str,
    models: &[oxplow_db::models::ModelSource],
    pages: &[ExtensionPage],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<RefKindDecl>, Vec<String>) {
    let block = key_line(manifest, "ref_kinds");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`ref_kinds` must be a list")],
        );
    };
    let mut out: Vec<RefKindDecl> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let f: RefKindFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, block, format!("ref kind: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "ref_kinds", "kind", &f.kind).or(block);
        let declared_at = at(file, line, "").trim_end_matches(": ").to_string();
        match decl_of(extension, models, pages, f, declared_at) {
            Ok(d) if out.iter().any(|o| o.kind == d.kind) => errors.push(at(
                file,
                line,
                format!("ref kind `{}` is declared twice", d.kind),
            )),
            Ok(d) if d.wikilink.is_some() && out.iter().any(|o| o.wikilink == d.wikilink) => errors
                .push(at(
                    file,
                    line,
                    format!(
                        "ref kind `{}`: `wikilink: {}` is another of its kinds' too",
                        d.kind,
                        d.wikilink.as_deref().unwrap_or_default()
                    ),
                )),
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn decl_of(
    extension: &str,
    models: &[oxplow_db::models::ModelSource],
    pages: &[ExtensionPage],
    f: RefKindFile,
    declared_at: String,
) -> Result<RefKindDecl, String> {
    let named = |m: String| format!("ref kind `{}`: {m}", f.kind);
    let prefix = format!(
        "{}_",
        oxplow_domain::events::schema::plugin_namespace(extension)
    );
    if !oxplow_domain::refs::grammar::is_valid_kind(&f.kind)
        || !f.kind.starts_with(&prefix)
        || f.kind.len() == prefix.len()
    {
        return Err(format!(
            "ref kind `{}` must be `{prefix}<name>`: lowercase letters, digits and `_`",
            f.kind
        ));
    }
    if !(f.id.starts_with('^') && f.id.ends_with('$')) {
        return Err(named(format!(
            "`id: {}` must be anchored (`^…$`): it matches a whole id",
            f.id
        )));
    }
    oxplow_domain::refs::kind::KindSpec::new(&f.kind, &f.id).map_err(|e| named(e.to_string()))?;
    let model = models
        .iter()
        .find(|m| m.decl.name == f.resolve)
        .ok_or_else(|| {
            named(format!(
                "`resolve: {}` isn't one of this extension's models (`models:`)",
                f.resolve
            ))
        })?;
    for col in ["ref", "title"] {
        if !model.decl.columns.iter().any(|c| c.name == col) {
            return Err(named(format!(
                "model `{}` has no `{col}` column (a ref kind's model gives each ref's title)",
                f.resolve
            )));
        }
    }
    let page = pages.iter().find(|p| p.id == f.page).ok_or_else(|| {
        named(format!(
            "`page: {}` isn't one of this extension's pages (`pages:`)",
            f.page
        ))
    })?;
    if let Some(w) = &f.wikilink {
        let core = oxplow_domain::refs::kind::core_kinds();
        if !oxplow_domain::refs::grammar::is_valid_kind(w) {
            return Err(named(format!(
                "`wikilink: {w}` must be lowercase letters, digits and `_`"
            )));
        }
        if core.get(w).is_some()
            || core.kind_for_wikilink_prefix(w).is_some()
            || RESERVED_PREFIXES.contains(&w.as_str())
        {
            return Err(named(format!("`wikilink: {w}` is oxplow's own")));
        }
    }
    if !REF_KIND_ICONS.contains(&f.icon.as_str()) {
        return Err(named(format!(
            "`icon: {}` isn't one oxplow draws; use one of {}",
            f.icon,
            REF_KIND_ICONS.join(", ")
        )));
    }
    Ok(RefKindDecl {
        extension: extension.to_string(),
        label: f.label,
        id_pattern: f.id,
        resolve: oxplow_db::models::extension_view(extension, &f.resolve),
        page: page.page_ref.clone(),
        wikilink: f.wikilink,
        icon: f.icon,
        declared_at,
        kind: f.kind,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::extensions::load_extensions;

    pub(crate) const MANIFEST: &str = "manifest: 2
name: acme
sharing: private
intent: { purpose: PRs., origin: null, examples: [] }
models:
  - name: prs
    version: 1
    description: Pull requests.
    columns:
      - { name: ref, type: TEXT, doc: The ref. }
      - { name: title, type: TEXT, doc: Its title. }
pages:
  - { id: pr, title: Pull request, category: Work, lens: open }
ref_kinds:
  - kind: acme_pr
    label: Pull request
    id: '^\\d+$'
    resolve: prs
    page: pr
    wikilink: pr
    icon: git-pull-request
";

    pub(crate) fn write_acme(root: &std::path::Path, manifest: &str) {
        let dir = root.join("oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::create_dir_all(dir.join("lenses")).unwrap();
        std::fs::write(dir.join("extension.yaml"), manifest).unwrap();
        std::fs::write(
            dir.join("models/prs.sql"),
            "SELECT 'acme_pr:1' AS ref, 'One' AS title",
        )
        .unwrap();
        std::fs::write(
            dir.join("lenses/open.yaml"),
            "title: Open\nquery: \"SELECT 1 AS n\"\n",
        )
        .unwrap();
    }

    fn acme(manifest: &str) -> crate::extensions::Extension {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), manifest);
        load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap()
    }

    /// The desktop draws exactly the icons a ref kind may name: its
    /// `REF_KIND_ICONS` map (`apps/desktop/src/refKinds.ts`) has a key for
    /// each, and no other.
    #[test]
    fn the_desktop_draws_every_allowed_icon() {
        let ts = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src/refKinds.ts"),
        )
        .unwrap();
        let start = ts.find("REF_KIND_ICONS").unwrap();
        let body = &ts[start..start + ts[start..].find("};").unwrap()];
        let mut drawn: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
            .collect();
        drawn.sort();
        let mut allowed = super::REF_KIND_ICONS.to_vec();
        allowed.sort();
        assert_eq!(drawn, allowed);
    }

    #[test]
    fn a_ref_kind_loads_with_its_model_and_page() {
        let ext = acme(MANIFEST);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let k = &ext.ref_kinds[0];
        assert_eq!(
            (k.kind.as_str(), k.resolve.as_str(), k.page.as_str()),
            ("acme_pr", "v_acme_prs", "page:ext.acme.pr")
        );
        assert_eq!(k.declared_at, "oxplow/extensions/acme/extension.yaml:15");
    }

    #[test]
    fn a_broken_ref_kind_is_an_error_at_its_line() {
        for (from, to, says) in [
            ("id: '^\\d+$'", "id: '^(\\d+$'", "bad id regex"),
            ("id: '^\\d+$'", "id: '\\d+'", "anchored"),
            ("kind: acme_pr", "kind: other_pr", "must be `acme_<name>`"),
            (
                "resolve: prs",
                "resolve: nope",
                "isn't one of this extension's models",
            ),
            (
                "page: pr",
                "page: nope",
                "isn't one of this extension's pages",
            ),
            ("wikilink: pr", "wikilink: git", "oxplow's own"),
            (
                "icon: git-pull-request",
                "icon: skull",
                "isn't one oxplow draws",
            ),
        ] {
            let ext = acme(&MANIFEST.replace(from, to));
            assert!(ext.ref_kinds.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains("extension.yaml:15:") && errors.contains(says),
                "{to}: {errors}"
            );
        }
    }
}
