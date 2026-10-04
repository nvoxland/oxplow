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
//!     searchable: found         # optional: one of its models, with `ref`, `title` and `body` (P9.D3)
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
    /// The view whose rows (`ref`, `title`, `body`) search indexes under
    /// the kind (`kind_search`); none, its refs aren't found by search.
    pub searchable: Option<String>,
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
    #[serde(default)]
    searchable: Option<String>,
    icon: String,
}

/// The registry's spec for `decl`: its id pattern and its `wikilink:`
/// sugar — the one constructor the vocabulary reactor and `plugin test`
/// register an extension's kinds with.
pub fn kind_spec(
    decl: &RefKindDecl,
) -> Result<oxplow_domain::refs::kind::KindSpec, oxplow_domain::refs::kind::KindError> {
    let spec = oxplow_domain::refs::kind::KindSpec::new(&decl.kind, &decl.id_pattern)?;
    Ok(match &decl.wikilink {
        Some(w) => spec.wikilink_prefix(w),
        None => spec,
    })
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

/// An id pattern both regex engines read the same and neither backtracks
/// on: the desktop matches it in JS (a backtracking engine) to resolve
/// `[[…]]`, oxplow in Rust (tsk797). So: characters, classes (`[…]`),
/// `.`, the shorthands `\d \w \s` (and their negations), escaped
/// punctuation, and quantifiers (`? * + {n} {n,m}`) — no groups, no
/// alternation, no other escapes (`\p{…}`, backreferences, flags).
fn portable_id_pattern(pattern: &str) -> Result<(), String> {
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('d' | 'D' | 'w' | 'W' | 's' | 'S') => {}
                Some(e) if e.is_ascii_punctuation() => {}
                Some(e) => {
                    return Err(format!(
                        "`\\{e}` isn't one both engines read alike; use `\\d`, `\\w`, `\\s`, a class or an escaped punctuation mark"
                    ))
                }
                None => return Err("it ends in a lone `\\`".into()),
            },
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' | ')' if !in_class => {
                return Err("no groups: an id pattern is a run of characters, classes and quantifiers".into())
            }
            '|' if !in_class => return Err("no alternation (`|`): use a class".into()),
            _ => {}
        }
    }
    if in_class {
        return Err("a class (`[`) isn't closed".into());
    }
    Ok(())
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
    portable_id_pattern(&f.id).map_err(|why| named(format!("`id: {}`: {why}", f.id)))?;
    oxplow_domain::refs::kind::KindSpec::new(&f.kind, &f.id).map_err(|e| named(e.to_string()))?;
    // A model of the extension's own with these columns, for `key:`.
    let model_with = |key: &str, name: &str, columns: &[&str], why: &str| {
        let model = models.iter().find(|m| m.decl.name == name).ok_or_else(|| {
            named(format!(
                "`{key}: {name}` isn't one of this extension's models (`models:`)"
            ))
        })?;
        match columns
            .iter()
            .find(|col| !model.decl.columns.iter().any(|c| c.name == **col))
        {
            Some(col) => Err(named(format!(
                "model `{name}` has no `{col}` column ({why})"
            ))),
            None => Ok(()),
        }
    };
    model_with(
        "resolve",
        &f.resolve,
        &["ref", "title"],
        "a ref kind's model gives each ref's title",
    )?;
    if let Some(searchable) = &f.searchable {
        model_with(
            "searchable",
            searchable,
            &["ref", "title", "body"],
            "what search indexes for each ref",
        )?;
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
        searchable: f
            .searchable
            .as_deref()
            .map(|m| oxplow_db::models::extension_view(extension, m)),
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
            ("id: '^\\d+$'", "id: '^(\\d+$'", "no groups"),
            // Patterns the renderer's JS engine would backtrack on, or read
            // differently from Rust's (tsk797).
            ("id: '^\\d+$'", "id: '^(a+)+$'", "no groups"),
            ("id: '^\\d+$'", "id: '^a|b$'", "no alternation"),
            ("id: '^\\d+$'", "id: '^\\p{L}+$'", "`\\p`"),
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

    /// P9.D3: `searchable:` names one of the extension's models with
    /// `ref`, `title` and `body` — what search indexes under the kind.
    #[test]
    fn a_searchable_kind_names_a_model_with_ref_title_and_body() {
        let found = "  - name: found
    version: 1
    description: Pull requests, as search finds them.
    columns:
      - { name: ref, type: TEXT, doc: The ref. }
      - { name: title, type: TEXT, doc: Its title. }
      - { name: body, type: TEXT, doc: Its description. }
pages:";
        let manifest = MANIFEST.replace("pages:", found).replace(
            "    wikilink: pr\n",
            "    wikilink: pr\n    searchable: found\n",
        );
        let load = |manifest: &str| {
            let dir = tempfile::tempdir().unwrap();
            write_acme(dir.path(), manifest);
            std::fs::write(
                dir.path().join("oxplow/extensions/acme/models/found.sql"),
                "SELECT 'acme_pr:1' AS ref, 'One' AS title, 'The first.' AS body",
            )
            .unwrap();
            load_extensions(dir.path())
                .into_iter()
                .find(|e| e.name == "acme")
                .unwrap()
        };
        let ext = load(&manifest);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.ref_kinds[0].searchable.as_deref(), Some("v_acme_found"));
        // Without it, a kind resolves and opens but isn't searched.
        assert_eq!(acme(MANIFEST).ref_kinds[0].searchable, None);
        for (to, says) in [
            (
                "searchable: nope",
                "`searchable: nope` isn't one of this extension's models",
            ),
            // `prs` has `ref` and `title`, no `body`.
            ("searchable: prs", "model `prs` has no `body` column"),
        ] {
            let ext = load(&manifest.replace("searchable: found", to));
            assert!(ext.ref_kinds.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(errors.contains(says), "{to}: {errors}");
        }
    }
}
