//! Project extensions and their lenses.
//!
//! An extension is a folder `oxplow/extensions/<name>/` in a stream's
//! worktree with an `extension.yaml` and `lenses/<slug>.yaml` files. A
//! lens is a query over the semantic layer (`v_*` views) plus how to
//! show it. See `.context/extensions.md`.
//!
//! Loading never fails as a whole: a broken extension or lens is
//! reported in that extension's `errors` and everything else still
//! loads.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{SemanticLayer, SqlCell, SqlQueryResult};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

/// Where project extensions live, relative to a worktree root.
pub const EXTENSIONS_DIR: &str = "oxplow/extensions";

/// How a lens renders its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LensViz {
    /// Rows and columns.
    Table,
    /// One line per row: the first column (or the first `columns` entry)
    /// is the headline, the rest are secondary.
    List,
    /// A single value: the first column of the first row.
    Number,
    /// The first column of the first row, rendered as markdown.
    Markdown,
}

/// A page a column value can link to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum LensLinkKind {
    /// `task:<id>`; the value is a task id.
    Task,
    /// `file:<path>`; the value is a repo-relative path.
    File,
    /// `wiki:<slug>`.
    Wiki,
    /// The effort's diff view; the value is an effort id.
    EffortDiff,
}

/// Makes a column's cells link to a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensLink {
    pub kind: LensLinkKind,
    /// Result column holding the target id. Defaults to the column itself.
    #[serde(default)]
    pub from: Option<String>,
}

/// How one result column is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensColumn {
    /// Result column name.
    pub key: String,
    /// Header text; defaults to `key`.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub link: Option<LensLink>,
}

/// A value the viewer (or an agent) can set when running the lens,
/// bound into the query as `:name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LensParam {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Used when the caller doesn't supply the param.
    #[serde(default)]
    pub default: Option<SqlCell>,
}

/// A lens file as written on disk (`lenses/<slug>.yaml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LensFile {
    title: String,
    #[serde(default)]
    description: String,
    query: String,
    #[serde(default = "default_viz")]
    viz: LensViz,
    #[serde(default)]
    params: Vec<LensParam>,
    #[serde(default)]
    columns: Vec<LensColumn>,
    /// Shown instead of an empty result.
    #[serde(default)]
    empty: Option<String>,
}

fn default_viz() -> LensViz {
    LensViz::Table
}

/// `extension.yaml` as written on disk.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionFile {
    name: String,
    #[serde(default)]
    description: String,
}

/// A loaded lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Lens {
    /// `<extension>/<slug>`: the stable id used by `lens:` page refs,
    /// `run_lens` and `get_lens`.
    pub id: String,
    pub extension: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub query: String,
    pub viz: LensViz,
    pub params: Vec<LensParam>,
    pub columns: Vec<LensColumn>,
    pub empty: Option<String>,
    /// Repo-relative path of the lens file.
    pub path: String,
}

/// A loaded extension and anything wrong with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub name: String,
    pub description: String,
    /// Repo-relative path of the extension folder.
    pub path: String,
    /// Problems found while loading; empty when healthy. A lens that
    /// failed to load is listed here and missing from `lenses`.
    pub errors: Vec<String>,
    pub lenses: Vec<Lens>,
}

/// The result of running a lens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensRun {
    pub lens: Lens,
    /// The parameter values actually used (supplied or default).
    pub params: BTreeMap<String, SqlCell>,
    pub result: SqlQueryResult,
}

/// Load every project extension under `root/oxplow/extensions/`.
/// Missing directory = no extensions.
pub fn load_extensions(root: &Path) -> Vec<Extension> {
    let Ok(entries) = std::fs::read_dir(root.join(EXTENSIONS_DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names.iter().map(|n| load_one(root, n)).collect()
}

/// Load one extension folder. Always returns an `Extension`; problems
/// go in `errors`.
fn load_one(root: &Path, name: &str) -> Extension {
    let rel = format!("{EXTENSIONS_DIR}/{name}");
    let dir = root.join(&rel);
    let mut ext = Extension {
        name: name.to_string(),
        description: String::new(),
        path: rel.clone(),
        errors: Vec::new(),
        lenses: Vec::new(),
    };

    let manifest = match std::fs::read_to_string(dir.join("extension.yaml")) {
        Ok(text) => text,
        Err(_) => {
            ext.errors.push(format!("{rel}: missing extension.yaml"));
            return ext;
        }
    };
    match serde_yaml::from_str::<ExtensionFile>(&manifest) {
        Ok(m) if m.name != name => {
            ext.errors.push(format!(
                "{rel}/extension.yaml: name `{}` must match its folder `{name}`",
                m.name
            ));
            return ext;
        }
        Ok(m) => ext.description = m.description,
        Err(e) => {
            ext.errors.push(format!("{rel}/extension.yaml: {e}"));
            return ext;
        }
    }

    let Ok(entries) = std::fs::read_dir(dir.join("lenses")) else {
        return ext;
    };
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| f.ends_with(".yaml") || f.ends_with(".yml"))
        .collect();
    files.sort();
    for file in files {
        let slug = file
            .trim_end_matches(".yaml")
            .trim_end_matches(".yml")
            .to_string();
        let lens_rel = format!("{rel}/lenses/{file}");
        let parsed = std::fs::read_to_string(root.join(&lens_rel))
            .map_err(|e| e.to_string())
            .and_then(|t| serde_yaml::from_str::<LensFile>(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(l) => ext.lenses.push(Lens {
                id: format!("{name}/{slug}"),
                extension: name.to_string(),
                slug,
                title: l.title,
                description: l.description,
                query: l.query,
                viz: l.viz,
                params: l.params,
                columns: l.columns,
                empty: l.empty,
                path: lens_rel,
            }),
            Err(e) => ext.errors.push(format!("{lens_rel}: {e}")),
        }
    }
    ext
}

/// Load the extension named `name`, if its folder exists.
fn load_named(root: &Path, name: &str) -> Result<Extension, DomainError> {
    if name.is_empty()
        || name.contains(['/', '\\', '.'])
        || !root.join(EXTENSIONS_DIR).join(name).is_dir()
    {
        return Err(DomainError::NotFound);
    }
    Ok(load_one(root, name))
}

/// Find one lens by `<extension>/<slug>`.
pub fn find_lens(root: &Path, id: &str) -> Result<Lens, DomainError> {
    let (ext, slug) = id.split_once('/').ok_or(DomainError::NotFound)?;
    load_named(root, ext)?
        .lenses
        .into_iter()
        .find(|l| l.slug == slug)
        .ok_or(DomainError::NotFound)
}

/// Run a lens: bind supplied params over defaults and query the
/// semantic layer. Unknown params are rejected, so a typo doesn't
/// silently fall back to a default.
pub async fn run_lens(
    layer: &SemanticLayer,
    root: &Path,
    id: &str,
    params: BTreeMap<String, SqlCell>,
) -> Result<LensRun, DomainError> {
    let lens = find_lens(root, id)?;
    execute(layer, lens, params).await
}

async fn execute(
    layer: &SemanticLayer,
    lens: Lens,
    supplied: BTreeMap<String, SqlCell>,
) -> Result<LensRun, DomainError> {
    if let Some(unknown) = supplied
        .keys()
        .find(|k| !lens.params.iter().any(|p| &p.name == *k))
    {
        let known: Vec<&str> = lens.params.iter().map(|p| p.name.as_str()).collect();
        return Err(DomainError::Invalid(format!(
            "lens {}: unknown param `{unknown}` (params: {})",
            lens.id,
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        )));
    }
    let mut params = BTreeMap::new();
    for p in &lens.params {
        let v = supplied
            .get(&p.name)
            .cloned()
            .or_else(|| p.default.clone())
            .unwrap_or(SqlCell::Null(()));
        params.insert(p.name.clone(), v);
    }
    let named: Vec<(String, SqlCell)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let result = layer
        .query_sql_named(&lens.query, named, None)
        .await
        .map_err(|e| match e {
            DomainError::Invalid(m) => DomainError::Invalid(format!("lens {}: {m}", lens.id)),
            other => other,
        })?;
    Ok(LensRun {
        lens,
        params,
        result,
    })
}

/// Load one extension and dry-run every lens with default params,
/// reporting query failures and `columns` keys the query doesn't
/// return as errors.
pub async fn validate_extension(
    layer: &SemanticLayer,
    root: &Path,
    name: &str,
) -> Result<Extension, DomainError> {
    let mut ext = load_named(root, name)?;
    for lens in ext.lenses.clone() {
        let id = lens.id.clone();
        match execute(layer, lens, BTreeMap::new()).await {
            Err(e) => ext
                .errors
                .push(e.to_string().replacen("invalid value: ", "", 1)),
            Ok(run) => {
                let cols = &run.result.columns;
                for c in &run.lens.columns {
                    let mut keys = vec![&c.key];
                    if let Some(from) = c.link.as_ref().and_then(|l| l.from.as_ref()) {
                        keys.push(from);
                    }
                    for k in keys {
                        if !cols.contains(k) {
                            ext.errors.push(format!(
                                "lens {id}: column `{k}` isn't in the query result (columns: {})",
                                cols.join(", ")
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use std::fs;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    const EXT: &str = "name: review\ndescription: Review helpers\n";
    const LENS: &str = r#"
title: Tasks by status
description: Every task with a given status.
params:
  - { name: status, label: Status, default: in_progress }
query: |
  SELECT id, title FROM v_task WHERE status = :status ORDER BY id
viz: table
columns:
  - { key: title, label: Task, link: { kind: task, from: id } }
empty: No tasks.
"#;

    async fn layer() -> SemanticLayer {
        SemanticLayer::new(Database::in_memory())
    }

    #[test]
    fn no_extensions_dir_means_no_extensions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_extensions(dir.path()).is_empty());
    }

    #[test]
    fn loads_an_extension_and_its_lenses() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );

        let exts = load_extensions(dir.path());
        assert_eq!(exts.len(), 1);
        let e = &exts[0];
        assert_eq!(e.name, "review");
        assert_eq!(e.description, "Review helpers");
        assert_eq!(e.path, "oxplow/extensions/review");
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.lenses.len(), 1);
        let l = &e.lenses[0];
        assert_eq!(l.id, "review/by-status");
        assert_eq!(l.slug, "by-status");
        assert_eq!(l.title, "Tasks by status");
        assert_eq!(l.viz, LensViz::Table);
        assert_eq!(l.path, "oxplow/extensions/review/lenses/by-status.yaml");
        assert_eq!(
            l.params[0].default,
            Some(SqlCell::Text("in_progress".into()))
        );
        assert_eq!(
            l.columns[0].link,
            Some(LensLink {
                kind: LensLinkKind::Task,
                from: Some("id".into())
            })
        );
    }

    #[test]
    fn a_broken_extension_reports_errors_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/good.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/bad.yaml",
            "title: x\nqueery: SELECT 1\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/nomanifest/lenses/a.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/misnamed/extension.yaml",
            "name: other\n",
        );

        let exts = load_extensions(dir.path());
        let by = |n: &str| {
            exts.iter()
                .find(|e| e.name == n)
                .unwrap_or_else(|| panic!("{n} missing"))
        };

        let review = by("review");
        assert_eq!(
            review
                .lenses
                .iter()
                .map(|l| l.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["good"]
        );
        assert_eq!(review.errors.len(), 1);
        assert!(
            review.errors[0].contains("lenses/bad.yaml"),
            "{:?}",
            review.errors
        );

        let nm = by("nomanifest");
        assert!(nm.errors[0].contains("extension.yaml"), "{:?}", nm.errors);
        assert!(nm.lenses.is_empty());

        let mis = by("misnamed");
        assert!(
            mis.errors[0].contains("must match its folder"),
            "{:?}",
            mis.errors
        );
    }

    #[test]
    fn find_lens_by_id() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        assert_eq!(
            find_lens(dir.path(), "review/by-status").unwrap().title,
            "Tasks by status"
        );
        assert!(matches!(
            find_lens(dir.path(), "review/nope"),
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            find_lens(dir.path(), "nope"),
            Err(DomainError::NotFound)
        ));
    }

    #[tokio::test]
    async fn run_lens_binds_defaults_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/echo.yaml",
            "title: Echo\nparams:\n  - { name: a, default: 1 }\n  - { name: b, default: two }\nquery: SELECT :a AS a, :b AS b\n",
        );
        let sl = layer().await;

        let run = run_lens(&sl, dir.path(), "review/echo", BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(run.result.columns, vec!["a", "b"]);
        assert_eq!(
            serde_json::to_value(&run.result.rows).unwrap(),
            serde_json::json!([[1, "two"]])
        );
        assert_eq!(run.params.get("b"), Some(&SqlCell::Text("two".into())));

        let mut over = BTreeMap::new();
        over.insert("b".to_string(), SqlCell::Text("three".into()));
        let run = run_lens(&sl, dir.path(), "review/echo", over)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&run.result.rows).unwrap(),
            serde_json::json!([[1, "three"]])
        );
    }

    #[tokio::test]
    async fn run_lens_rejects_unknown_params_and_reports_query_errors() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/broken.yaml",
            "title: Broken\nquery: SELECT nope FROM v_nothing\n",
        );
        let sl = layer().await;

        let mut bad = BTreeMap::new();
        bad.insert("stauts".to_string(), SqlCell::Text("done".into()));
        let err = run_lens(&sl, dir.path(), "review/by-status", bad)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("stauts")),
            "{err:?}"
        );

        let err = run_lens(&sl, dir.path(), "review/broken", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("review/broken")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn validate_dry_runs_every_lens() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "oxplow/extensions/review/extension.yaml", EXT);
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/by-status.yaml",
            LENS,
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/broken.yaml",
            "title: Broken\nquery: SELECT nope FROM v_nothing\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/review/lenses/badcol.yaml",
            "title: Bad column\nquery: SELECT 1 AS n\ncolumns:\n  - { key: missing }\n",
        );
        let sl = layer().await;

        let e = validate_extension(&sl, dir.path(), "review").await.unwrap();
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors.iter().any(|m| m.contains("review/broken")));
        assert!(e
            .errors
            .iter()
            .any(|m| m.contains("review/badcol") && m.contains("missing")));

        assert!(matches!(
            validate_extension(&sl, dir.path(), "nope").await,
            Err(DomainError::NotFound)
        ));
    }
}
