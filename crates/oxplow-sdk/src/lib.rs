//! The plugin SDK (`.context/extensions.md` "The SDK";
//! `.context/target-architecture.md` §10.5): scaffold, check and migrate
//! an extension.
//!
//! One library behind three callers — the `oxplow plugin new|check|migrate`
//! CLI in the Tauri binary, the RPC/MCP `validate_extension`, and
//! `save_lens` — so every message reads the same (`file:line: what — fix`)
//! wherever the author meets it. For an AI author that consistency is the
//! feature: the skill says "run `check` after every edit", and the error
//! it gets back names the file and line and says what to change.

use std::path::{Path, PathBuf};

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_app::extensions::{self, Extension, EXTENSIONS_DIR};
use oxplow_db::SemanticLayer;
use oxplow_domain::DomainError;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("{0}")]
    Invalid(String),
    #[error("no extension `{0}` under {EXTENSIONS_DIR}/")]
    NotFound(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Domain(#[from] DomainError),
}

/// What `plugin new` can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An extension folder with a v2 manifest and one starter lens.
    Lens,
    /// An extension folder with a v2 manifest only.
    Extension,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "lens" => Some(Kind::Lens),
            "extension" => Some(Kind::Extension),
            _ => None,
        }
    }
}

/// What `scaffold` wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scaffolded {
    pub name: String,
    /// The extension folder, relative to the root.
    pub dir: String,
    /// Every file written, relative to the root.
    pub files: Vec<String>,
}

/// Create `oxplow/extensions/<name>/` with a v2 manifest carrying an
/// `intent` (its `origin` = the thread/effort ref that asked for it), one
/// example and a matching fixture, and — for a lens — one starter lens the
/// example names. Refuses an existing folder, a bad name, or a non-ref
/// origin; `check` passes on what it writes.
pub fn scaffold(
    root: &Path,
    kind: Kind,
    name: &str,
    origin: Option<&str>,
) -> Result<Scaffolded, SdkError> {
    if !extensions::is_valid_name(name) {
        return Err(SdkError::Invalid(format!(
            "`{name}` must be lowercase letters, digits and single dashes (e.g. `review-notes`)"
        )));
    }
    if let Some(o) = origin {
        oxplow_domain::refs::grammar::CanonicalRef::parse(o).map_err(|e| {
            SdkError::Invalid(format!(
                "--origin `{o}` is not a canonical ref (`effort:eff42`, `thread:thr3`): {}",
                e.reason()
            ))
        })?;
    }
    let rel_dir = format!("{EXTENSIONS_DIR}/{name}");
    let dir = root.join(&rel_dir);
    if dir.exists() {
        return Err(SdkError::Invalid(format!(
            "{rel_dir} already exists; pick another name or edit it in place"
        )));
    }
    let mut files = Vec::new();
    let mut write = |rel: &str, body: String| -> Result<(), SdkError> {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
        files.push(rel.to_string());
        Ok(())
    };
    let (example_input, example_expect) = match kind {
        Kind::Lens => (
            format!("{{ lens: {name}, params: {{ stream_id: 1 }} }}"),
            "one row per open task in the stream, newest first".to_string(),
        ),
        Kind::Extension => ("{}".to_string(), "TODO: what a run should show".to_string()),
    };
    write(
        &format!("{rel_dir}/extension.yaml"),
        extensions::scaffold_manifest(&extensions::ManifestScaffold {
            name,
            description: "TODO: one line on what this extension shows or does",
            purpose: "TODO: the question this answers, or the job it does",
            origin,
            example_name: "basic",
            example_input: &example_input,
            example_expect: &example_expect,
        }),
    )?;
    write(
        &format!("{rel_dir}/fixtures/basic.yaml"),
        format!(
            "# The acceptance example from extension.yaml as a fixture for `oxplow plugin test`\n\
             # (input in, expected output out). Keep the two in step.\n\
             name: basic\ninput: {example_input}\nexpect: {example_expect}\n"
        ),
    )?;
    if kind == Kind::Lens {
        write(
            &format!("{rel_dir}/lenses/{name}.yaml"),
            format!(
                "title: {title}\n\
                 description: \"TODO: what this lens answers.\"\n\
                 params:\n\
                 \x20 # Filled in with the viewer's stream unless a value is given.\n\
                 \x20 - {{ name: stream_id, label: Stream }}\n\
                 query: |\n\
                 \x20 SELECT id, title, status, updated_at\n\
                 \x20 FROM v_task\n\
                 \x20 WHERE stream_id = :stream_id AND status IN ('ready', 'in_progress', 'blocked')\n\
                 \x20 ORDER BY updated_at DESC\n\
                 viz: table\n\
                 columns:\n\
                 \x20 - {{ key: title, label: Task, link: {{ kind: task, from: id }} }}\n\
                 \x20 - {{ key: status }}\n\
                 empty: No open tasks in this stream.\n\
                 launcher: {{ category: Work }}\n",
                title = title_case(name)
            ),
        )?;
    }
    Ok(Scaffolded {
        name: name.to_string(),
        dir: rel_dir,
        files,
    })
}

fn title_case(name: &str) -> String {
    name.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What `check` found.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub name: String,
    /// No errors (warnings don't fail a check).
    pub ok: bool,
    /// `file:line: what — fix` lines.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Whether every lens and advisory was dry-run against a database.
    pub sql_checked: bool,
    pub extension: Extension,
}

/// Load `name` under `root` and report everything wrong with it: the
/// manifest's shape and lifecycle, every cross-reference, every lens's
/// shape, and — with a `layer` — a dry run of every lens and advisory
/// against the semantic layer. This is what `validate_extension` returns
/// and what `oxplow plugin check` prints.
pub async fn check(
    root: &Path,
    name: &str,
    catalog: &ExtensionCatalog,
    layer: Option<&SemanticLayer>,
) -> Result<CheckReport, SdkError> {
    let extension = match layer {
        Some(layer) => extensions::validate_extension(layer, catalog, root, name).await,
        None => catalog.named(root, name),
    }
    .map_err(|e| match e {
        DomainError::NotFound => SdkError::NotFound(name.to_string()),
        other => SdkError::Domain(other),
    })?;
    Ok(CheckReport {
        name: name.to_string(),
        ok: extension.errors.is_empty(),
        errors: extension.errors.clone(),
        warnings: extension.warnings.clone(),
        sql_checked: layer.is_some(),
        extension,
    })
}

/// `check` for a folder path (`oxplow/extensions/<name>` or an absolute
/// path to it) or a bare name: the name is its last component.
pub fn name_of(path_or_name: &str) -> &str {
    Path::new(path_or_name.trim_end_matches('/'))
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path_or_name)
}

/// How `render_findings` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One finding per line: `error: <file:line: what — fix>`.
    Text,
    Json,
}

/// The report as text or JSON. Text is what an agent reads back from the
/// CLI; JSON is for tooling.
pub fn render_findings(report: &CheckReport, format: Format) -> String {
    match format {
        Format::Json => serde_json::to_string_pretty(report).expect("report serializes"),
        Format::Text => {
            let mut out = String::new();
            for e in &report.errors {
                out.push_str("error: ");
                out.push_str(e);
                out.push('\n');
            }
            for w in &report.warnings {
                out.push_str("warning: ");
                out.push_str(w);
                out.push('\n');
            }
            let sql = if report.sql_checked {
                "lenses and advisories dry-run against the project's database"
            } else {
                "no project database found, so lens SQL was not dry-run (open the project in oxplow or use validate_extension)"
            };
            out.push_str(&format!(
                "{}: {} error{}, {} warning{}; {sql}\n",
                report.name,
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" },
                report.warnings.len(),
                if report.warnings.len() == 1 { "" } else { "s" },
            ));
            out
        }
    }
}

/// What `migrate` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Migrated {
    pub name: String,
    pub path: String,
    /// False when the manifest was already v2 (nothing written).
    pub changed: bool,
}

/// Rewrite `oxplow/extensions/<name>/extension.yaml` from v1 to v2 with
/// the textual migration the loader already applies in memory
/// (`extensions::migrate_v1`). Idempotent; a person's consent survives it.
pub fn migrate(root: &Path, name: &str) -> Result<Migrated, SdkError> {
    let rel = format!("{EXTENSIONS_DIR}/{name}/extension.yaml");
    let path = root.join(&rel);
    let text = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => SdkError::NotFound(name.to_string()),
        _ => SdkError::Io(e),
    })?;
    let migrated = extensions::migrate_v1::migrate_v1_to_v2(&text);
    let changed = migrated != text;
    if changed {
        std::fs::write(&path, migrated)?;
    }
    Ok(Migrated {
        name: name.to_string(),
        path: rel,
        changed,
    })
}

/// The project's local database, when the folder is an oxplow project
/// that has been opened: what `check` dry-runs lens SQL against.
pub fn project_database(root: &Path) -> Option<PathBuf> {
    let path = root.join(".oxplow").join("local.sqlite");
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[tokio::test]
    async fn new_lens_then_check_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let made = scaffold(dir.path(), Kind::Lens, "demo", Some("effort:eff42")).unwrap();
        assert_eq!(made.dir, "oxplow/extensions/demo");
        assert_eq!(
            made.files,
            vec![
                "oxplow/extensions/demo/extension.yaml",
                "oxplow/extensions/demo/fixtures/basic.yaml",
                "oxplow/extensions/demo/lenses/demo.yaml",
            ]
        );
        let report = check(dir.path(), "demo", &ExtensionCatalog::new(), None)
            .await
            .unwrap();
        assert!(report.ok, "{}", render_findings(&report, Format::Text));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!report.sql_checked);
        assert_eq!(report.extension.manifest_version, 2);
        assert_eq!(
            report.extension.intent.as_ref().unwrap().origin.as_deref(),
            Some("effort:eff42")
        );
        assert_eq!(report.extension.lenses.len(), 1);
        assert_eq!(report.extension.lenses[0].title, "Demo");
        let text = render_findings(&report, Format::Text);
        assert!(text.contains("demo: 0 errors, 0 warnings"), "{text}");
        // A second scaffold refuses to overwrite; bad names and origins refuse too.
        assert!(scaffold(dir.path(), Kind::Lens, "demo", None).is_err());
        assert!(scaffold(dir.path(), Kind::Lens, "Bad Name", None).is_err());
        assert!(scaffold(dir.path(), Kind::Lens, "other", Some("nope")).is_err());
        let ext_only = scaffold(dir.path(), Kind::Extension, "bare", None).unwrap();
        assert_eq!(ext_only.files.len(), 2);
        assert!(
            check(dir.path(), "bare", &ExtensionCatalog::new(), None)
                .await
                .unwrap()
                .ok
        );
    }

    #[tokio::test]
    async fn check_reports_lifecycle_errors_with_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/team/extension.yaml",
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nref_kinds:\n  - kind: ticket\n",
        );
        let report = check(dir.path(), "team", &ExtensionCatalog::new(), None)
            .await
            .unwrap();
        assert!(!report.ok);
        let text = render_findings(&report, Format::Text);
        assert!(
            text.contains(
                "error: oxplow/extensions/team/extension.yaml:8: `ref_kinds` is experimental"
            ),
            "{text}"
        );
        assert!(text.contains("team: 1 error, 0 warnings"), "{text}");
        let json: serde_json::Value =
            serde_json::from_str(&render_findings(&report, Format::Json)).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["errors"].as_array().unwrap().len(), 1);
        assert!(matches!(
            check(dir.path(), "nope", &ExtensionCatalog::new(), None).await,
            Err(SdkError::NotFound(_))
        ));
    }

    #[test]
    fn migrate_rewrites_v1_once() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/old/extension.yaml",
            "name: old\ndescription: Old one\nslots: []\n",
        );
        let first = migrate(dir.path(), "old").unwrap();
        assert!(first.changed);
        let text = std::fs::read_to_string(dir.path().join(&first.path)).unwrap();
        assert!(text.starts_with("manifest: 2\nname: old\n"), "{text}");
        assert!(text.contains("slot_mounts: []"), "{text}");
        let second = migrate(dir.path(), "old").unwrap();
        assert!(!second.changed);
        assert!(matches!(
            migrate(dir.path(), "nope"),
            Err(SdkError::NotFound(_))
        ));
        assert_eq!(name_of("oxplow/extensions/old/"), "old");
        assert_eq!(name_of("old"), "old");
    }
}
