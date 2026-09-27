//! Per-function analysis of a change: tree-sitter metrics for both sides
//! of each file, per-function churn, and import deltas zoned by the
//! project's zone rules. Shared by the `analyze_functions_at_refs` IPC and
//! the change-analysis producer.

use oxplow_code_deps::{
    diff_edges, extract_imports, ImportEdge, ZoneRules, ZonedImportEdge, ZONE_EXTERNAL,
};
use oxplow_code_metrics::{analyze_file, FunctionMetrics, Visibility};
use serde::{Deserialize, Serialize};
use specta::Type;

/// One file's content at one side of the diff. `content == None` means
/// the file did not exist on that side (e.g. add/delete).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AnalyzeFileSpec {
    pub path: String,
    pub base_content: Option<String>,
    pub head_content: Option<String>,
}

/// Function metadata for one (path, side) pair.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFunction {
    pub name: String,
    pub start_line: u32,
    pub length: u32,
    pub complexity: f64,
    pub parameter_count: u32,
    pub nloc: u32,
    /// Outer-to-inner names of the named-declaration ancestors this
    /// function lives inside (class / impl / module / namespace).
    /// Empty for top-level functions; used to render the Functions
    /// card hierarchically.
    pub container_path: Vec<String>,
    /// Heuristic public/private classification — see
    /// `oxplow_code_metrics::Visibility`. Frontend uses this to
    /// drive a "Show private" filter on the Semantic view.
    /// Serialized as `"public"` / `"private"` / `"unknown"`.
    pub visibility: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileSide {
    pub path: String,
    /// `"base"` or `"head"`.
    pub side: String,
    pub functions: Vec<AnalyzedFunction>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFunctionChurn {
    pub name: String,
    pub container_path: Vec<String>,
    pub start_line_head: u32,
    pub added_lines: u32,
    pub deleted_lines: u32,
    pub modified_lines: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileChurn {
    pub path: String,
    pub file_added: u32,
    pub file_deleted: u32,
    pub functions: Vec<AnalyzedFunctionChurn>,
}

/// Delta between the before- and after-revision import edges for a
/// single file. `cross_zone_added` is the highlight signal — a new
/// import that crosses an architectural zone boundary (e.g. `ui`
/// suddenly reaches into `store`) is the "wrong layer" callout.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportDelta {
    pub path: String,
    pub added: Vec<ZonedImportEdge>,
    pub removed: Vec<ZonedImportEdge>,
    /// Subset of `added` whose `from_zone != to_zone` AND `to_zone`
    /// is known (we never flag external/unresolved targets).
    pub cross_zone_added: Vec<ZonedImportEdge>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzeFunctionsResult {
    pub sides: Vec<AnalyzedFileSide>,
    /// One entry per file with both base + head content present —
    /// i.e. modified files. Added / deleted / unsupported / binary
    /// files are omitted (the file-level totals already cover those
    /// cases via `BranchChangeEntry.additions` / `deletions`).
    #[serde(default)]
    pub churn: Vec<AnalyzedFileChurn>,
    /// One entry per file with imports that changed (added or
    /// removed). Files with stable imports are omitted.
    #[serde(default)]
    pub import_deltas: Vec<ImportDelta>,
}

pub fn analyze_files(files: Vec<AnalyzeFileSpec>, zones: &ZoneRules) -> AnalyzeFunctionsResult {
    let mut sides: Vec<AnalyzedFileSide> = Vec::new();
    let mut churn: Vec<AnalyzedFileChurn> = Vec::new();
    let mut import_deltas: Vec<ImportDelta> = Vec::new();
    for spec in files {
        // Run analyze_file once per side (working metrics for churn
        // attribution — we don't want to re-parse).
        let base_metrics = spec
            .base_content
            .as_deref()
            .map(|c| analyze_file(&spec.path, c))
            .unwrap_or_default();
        let head_metrics = spec
            .head_content
            .as_deref()
            .map(|c| analyze_file(&spec.path, c))
            .unwrap_or_default();

        if spec.base_content.is_some() {
            sides.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "base".into(),
                functions: to_analyzed(base_metrics.clone()),
            });
        }
        if spec.head_content.is_some() {
            sides.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "head".into(),
                functions: to_analyzed(head_metrics.clone()),
            });
        }

        if let (Some(base), Some(head)) =
            (spec.base_content.as_deref(), spec.head_content.as_deref())
        {
            let fc = crate::churn::compute_file_churn(
                &spec.path,
                &base_metrics,
                &head_metrics,
                base,
                head,
            );
            churn.push(AnalyzedFileChurn {
                path: fc.path,
                file_added: fc.file_added,
                file_deleted: fc.file_deleted,
                functions: fc
                    .functions
                    .into_iter()
                    .map(|f| AnalyzedFunctionChurn {
                        name: f.name,
                        container_path: f.container_path,
                        start_line_head: f.start_line_head,
                        added_lines: f.added_lines,
                        deleted_lines: f.deleted_lines,
                        modified_lines: f.modified_lines,
                    })
                    .collect(),
            });

            // Import delta on this file. We extract both sides and
            // diff by (kind, module). Each edge gets zoned via the
            // path-based resolver — for now a tiny built-in
            // (Rust crate-name lookup + obvious external/relative
            // shortcuts), with unresolved edges marked to_zone=None
            // so they never contribute to `cross_zone_added`.
            let base_edges = extract_imports(&spec.path, base);
            let head_edges = extract_imports(&spec.path, head);
            let (added_raw, removed_raw) = diff_edges(&base_edges, &head_edges);
            if !added_raw.is_empty() || !removed_raw.is_empty() {
                let added: Vec<ZonedImportEdge> =
                    added_raw.into_iter().map(|e| zone_edge(e, zones)).collect();
                let removed: Vec<ZonedImportEdge> = removed_raw
                    .into_iter()
                    .map(|e| zone_edge(e, zones))
                    .collect();
                let cross_zone_added: Vec<ZonedImportEdge> = added
                    .iter()
                    .filter(|z| z.is_cross_zone())
                    .cloned()
                    .collect();
                import_deltas.push(ImportDelta {
                    path: spec.path.clone(),
                    added,
                    removed,
                    cross_zone_added,
                });
            }
        }
    }
    sides.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.side.cmp(&b.side)));
    churn.sort_by(|a, b| a.path.cmp(&b.path));
    import_deltas.sort_by(|a, b| a.path.cmp(&b.path));
    AnalyzeFunctionsResult {
        sides,
        churn,
        import_deltas,
    }
}

/// Resolve an [`ImportEdge`] to a [`ZonedImportEdge`]. The resolver
/// is intentionally minimal for v1:
///
/// - Rust `use foo::bar`: take the first path segment as a crate
///   name. `crate` / `self` / `super` map back to the importer's
///   own zone (same-zone). Other names go through
///   [`ZoneRules::zone_for_module`], which looks the name up in the
///   project's own zone patterns; no hit means the target is
///   `external` (a real crate we don't host).
/// - TS/JS `import "./foo"` / `"../bar"`: relative paths join with
///   the importer's directory. The joined path goes through the
///   path zone classifier. Non-relative ("react", "@scope/pkg")
///   marks as `External`.
/// - Everything else: unresolved (to_zone = None), so cross-zone
///   logic ignores it. Better to underflag than overflag.
fn zone_edge(edge: ImportEdge, zones: &ZoneRules) -> ZonedImportEdge {
    if let Some(target) = resolve_target(&edge, zones) {
        match target {
            ResolveResult::RepoPath(path) => zones.zone_for_resolved_edge(edge, &path),
            ResolveResult::Zone(zone) => {
                let from_zone = zones.classify(&edge.from_path);
                ZonedImportEdge {
                    edge,
                    from_zone,
                    to_zone: Some(zone),
                }
            }
            ResolveResult::External => {
                // Build a synthetic edge whose to_zone is External.
                let from_zone = zones.classify(&edge.from_path);
                ZonedImportEdge {
                    edge,
                    from_zone,
                    to_zone: Some(ZONE_EXTERNAL.to_string()),
                }
            }
        }
    } else {
        zones.zone_for_unresolved_edge(edge)
    }
}

enum ResolveResult {
    /// In-repo file path.
    RepoPath(String),
    /// A zone resolved directly from a module name (no file path
    /// involved) — see `ZoneRules::zone_for_module`.
    Zone(String),
    /// Definitely not in this repo (system lib, npm package, etc.).
    External,
}

fn resolve_target(edge: &ImportEdge, zones: &ZoneRules) -> Option<ResolveResult> {
    use oxplow_code_deps::ImportKind;
    match edge.kind {
        ImportKind::Use => resolve_rust(edge, zones),
        ImportKind::Import => resolve_ts_like(edge),
        ImportKind::PyImport
        | ImportKind::GoImport
        | ImportKind::JavaImport
        | ImportKind::Include
        | ImportKind::Using
        | ImportKind::CljRequire => None,
    }
}

fn resolve_rust(edge: &ImportEdge, zones: &ZoneRules) -> Option<ResolveResult> {
    let first = edge.module.split("::").next().unwrap_or("");
    if first.is_empty() {
        return None;
    }
    if matches!(first, "crate" | "self" | "super") {
        // Resolves back inside the importer's own crate — same zone
        // by construction.
        return Some(ResolveResult::RepoPath(edge.from_path.clone()));
    }
    if let Some(zone) = zones.zone_for_module(first) {
        return Some(ResolveResult::Zone(zone));
    }
    Some(ResolveResult::External)
}

fn resolve_ts_like(edge: &ImportEdge) -> Option<ResolveResult> {
    let module = edge.module.trim();
    if module.starts_with("./") || module.starts_with("../") {
        let from_dir = std::path::Path::new(&edge.from_path).parent()?;
        let joined = from_dir.join(module);
        // Lexical normalization — collapse `..` and `.`. We can't
        // touch the filesystem from here (callers may be analyzing
        // a git-ref content). Filesystem-aware resolution can come
        // later if the heuristic is wrong too often.
        let normalized = normalize_relative_path(&joined);
        Some(ResolveResult::RepoPath(normalized))
    } else {
        // Bare specifier ("react", "@scope/x", "node:fs") → external.
        Some(ResolveResult::External)
    }
}

fn normalize_relative_path(path: &std::path::Path) -> String {
    let mut out: Vec<String> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => {
                out.push(other.as_os_str().to_string_lossy().into_owned());
            }
        }
    }
    out.join("/")
}

fn to_analyzed(metrics: Vec<FunctionMetrics>) -> Vec<AnalyzedFunction> {
    metrics
        .into_iter()
        .map(|m| AnalyzedFunction {
            name: m.name,
            start_line: m.start_line,
            length: m.length,
            complexity: m.complexity as f64,
            parameter_count: m.parameter_count,
            // We don't compute non-comment line count separately;
            // approximate as length. Renderer treats it as informational.
            nloc: m.length,
            container_path: m.container_path,
            visibility: match m.visibility {
                Visibility::Public => "public",
                Visibility::Private => "private",
                Visibility::Unknown => "unknown",
            }
            .to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_code_deps::{ZONE_EXTERNAL, ZONE_OTHER};

    /// A project zone table for the import tests — the same shape a
    /// project writes into `.oxplow/project.yaml` (tsk251). Oxplow has no
    /// built-in table, so every zone assertion below is against THESE
    /// rules, not oxplow's opinion of the repo.
    fn zones() -> ZoneRules {
        let config: Vec<oxplow_config::ZoneRuleConfig> = [
            ("analysis", "crates/oxplow-code-deps/**"),
            ("store", "crates/oxplow-db/**"),
        ]
        .into_iter()
        .map(|(zone, pattern)| oxplow_config::ZoneRuleConfig {
            patterns: vec![pattern.to_string()],
            zone: zone.to_string(),
            color: None,
        })
        .collect();
        ZoneRules::from_config(&config)
    }

    fn spec(path: &str, base: Option<&str>, head: Option<&str>) -> AnalyzeFileSpec {
        AnalyzeFileSpec {
            path: path.into(),
            base_content: base.map(str::to_string),
            head_content: head.map(str::to_string),
        }
    }

    #[test]
    fn each_side_gets_its_functions() {
        let r = analyze_files(
            vec![spec(
                "src/foo.rs",
                Some("fn a() {}"),
                Some("fn a() { if true { 1; } }"),
            )],
            &zones(),
        );
        assert_eq!(r.sides.len(), 2);
        let head = r.sides.iter().find(|s| s.side == "head").unwrap();
        assert_eq!(head.functions.len(), 1);
        assert!(head.functions[0].complexity >= 2.0);
    }

    #[test]
    fn an_added_file_has_only_a_head_side() {
        let r = analyze_files(
            vec![spec("src/new.py", None, Some("def f(x):\n    return x\n"))],
            &zones(),
        );
        assert_eq!(r.sides.len(), 1);
        assert_eq!(r.sides[0].side, "head");
    }

    #[test]
    fn an_added_import_into_another_zone_is_cross_zone() {
        let r = analyze_files(
            vec![spec(
                "crates/oxplow-code-deps/src/lib.rs",
                Some("use std::fs;\nfn a() {}\n"),
                Some("use std::fs;\nuse oxplow_db::Database;\nfn a() {}\n"),
            )],
            &zones(),
        );
        assert_eq!(r.import_deltas.len(), 1);
        let cz = &r.import_deltas[0].cross_zone_added;
        assert!(!cz.is_empty(), "expected cross-zone added; got {r:?}");
        assert_eq!(cz[0].from_zone, "analysis");
        assert_eq!(cz[0].to_zone.as_deref(), Some("store"));
    }

    /// No `zones:` block means no zone vocabulary, so nothing is
    /// cross-zone; the import delta itself is still reported.
    #[test]
    fn without_a_zone_table_nothing_is_cross_zone() {
        let r = analyze_files(
            vec![spec(
                "crates/oxplow-code-deps/src/lib.rs",
                Some("use std::fs;\nfn a() {}\n"),
                Some("use std::fs;\nuse oxplow_db::Database;\nfn a() {}\n"),
            )],
            &ZoneRules::from_config(&[]),
        );
        let delta = &r.import_deltas[0];
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].from_zone, ZONE_OTHER);
        assert!(delta.cross_zone_added.is_empty());
    }

    /// A store crate pulling in serde is not a layer violation.
    #[test]
    fn an_external_import_is_not_cross_zone() {
        let r = analyze_files(
            vec![spec(
                "crates/oxplow-db/src/lib.rs",
                Some("fn a() {}\n"),
                Some("use serde::Serialize;\nfn a() {}\n"),
            )],
            &zones(),
        );
        let delta = &r.import_deltas[0];
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].to_zone.as_deref(), Some(ZONE_EXTERNAL));
        assert!(delta.cross_zone_added.is_empty());
    }

    /// Unsupported languages still get (empty) sides, so the caller can
    /// see the file was looked at.
    #[test]
    fn unsupported_languages_get_empty_sides() {
        let r = analyze_files(
            vec![spec("README.md", Some("# old"), Some("# new"))],
            &zones(),
        );
        assert_eq!(r.sides.len(), 2);
        assert!(r.sides[0].functions.is_empty());
    }

    #[test]
    fn a_modified_file_gets_its_functions_and_churn() {
        // The head changes alpha's body AND adds beta.
        let r = analyze_files(
            vec![spec(
                "src/x.rs",
                Some("fn alpha() -> i32 {\n    1\n}\n"),
                Some("fn alpha() -> i32 {\n    2\n}\n\nfn beta() -> i32 {\n    3\n}\n"),
            )],
            &ZoneRules::from_config(&[]),
        );
        let side = |name: &str| r.sides.iter().find(|s| s.side == name).unwrap();
        assert!(side("base").functions.iter().any(|f| f.name == "alpha"));
        assert!(side("head").functions.iter().any(|f| f.name == "alpha"));
        assert!(side("head").functions.iter().any(|f| f.name == "beta"));
        assert_eq!(r.churn.len(), 1, "one modified file → one churn entry");
        assert_eq!(r.churn[0].path, "src/x.rs");
        assert!(
            r.churn[0].file_added > 0,
            "adding beta adds lines: {:?}",
            r.churn[0]
        );
    }

    #[test]
    fn an_added_file_has_no_churn() {
        let r = analyze_files(
            vec![spec("src/new.rs", None, Some("fn brand_new() {}\n"))],
            &ZoneRules::from_config(&[]),
        );
        assert!(r.sides[0].functions.iter().any(|f| f.name == "brand_new"));
        assert!(r.churn.is_empty(), "no base content, no before→after churn");
    }
}
