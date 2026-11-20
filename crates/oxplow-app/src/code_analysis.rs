//! Per-function analysis of a change: tree-sitter metrics for both sides
//! of each file, per-function churn, and the imports each file added or
//! removed. Shared by the `analyze_functions_at_refs` IPC and the
//! change-analysis producer.

use oxplow_code_deps::{diff_edges, extract_imports, ImportEdge};
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
    /// Its symbol id within the file (`Parser::parse(String,int)`).
    pub id: String,
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
    pub id: String,
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

/// The import edges a single file added or removed between the before-
/// and after-revision, as written (module strings unresolved).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportDelta {
    pub path: String,
    pub added: Vec<ImportEdge>,
    pub removed: Vec<ImportEdge>,
}

/// One changed line range of a file; see `churn::Hunk`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedHunk {
    pub head_start: Option<u32>,
    pub head_end: Option<u32>,
    pub base_start: Option<u32>,
    pub base_end: Option<u32>,
    pub head_after: Option<u32>,
}

/// The changed line ranges of one file.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileHunks {
    pub path: String,
    pub hunks: Vec<AnalyzedHunk>,
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
    /// One entry per file with changed lines, added and deleted files
    /// included (a missing side is empty).
    #[serde(default)]
    pub hunks: Vec<AnalyzedFileHunks>,
}

pub fn analyze_files(
    languages: &oxplow_code_metrics::Languages,
    files: Vec<AnalyzeFileSpec>,
) -> AnalyzeFunctionsResult {
    let mut sides: Vec<AnalyzedFileSide> = Vec::new();
    let mut churn: Vec<AnalyzedFileChurn> = Vec::new();
    let mut import_deltas: Vec<ImportDelta> = Vec::new();
    let mut hunks: Vec<AnalyzedFileHunks> = Vec::new();
    for spec in files {
        // Run analyze_file once per side (working metrics for churn
        // attribution — we don't want to re-parse).
        let base_metrics = spec
            .base_content
            .as_deref()
            .map(|c| analyze_file(languages, &spec.path, c))
            .unwrap_or_default();
        let head_metrics = spec
            .head_content
            .as_deref()
            .map(|c| analyze_file(languages, &spec.path, c))
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

        if spec.base_content.is_some() || spec.head_content.is_some() {
            let found = crate::churn::diff_hunks(
                spec.base_content.as_deref().unwrap_or(""),
                spec.head_content.as_deref().unwrap_or(""),
            );
            if !found.is_empty() {
                hunks.push(AnalyzedFileHunks {
                    path: spec.path.clone(),
                    hunks: found
                        .into_iter()
                        .map(|h| AnalyzedHunk {
                            head_start: h.head.map(|r| r.0),
                            head_end: h.head.map(|r| r.1),
                            base_start: h.base.map(|r| r.0),
                            base_end: h.base.map(|r| r.1),
                            head_after: h.head_after,
                        })
                        .collect(),
                });
            }
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
                        id: f.id,
                        name: f.name,
                        container_path: f.container_path,
                        start_line_head: f.start_line_head,
                        added_lines: f.added_lines,
                        deleted_lines: f.deleted_lines,
                        modified_lines: f.modified_lines,
                    })
                    .collect(),
            });

            // Import delta on this file: both sides' edges, diffed by
            // (kind, module).
            let base_edges = extract_imports(languages, &spec.path, base);
            let head_edges = extract_imports(languages, &spec.path, head);
            let (added, removed) = diff_edges(&base_edges, &head_edges);
            if !added.is_empty() || !removed.is_empty() {
                import_deltas.push(ImportDelta {
                    path: spec.path.clone(),
                    added,
                    removed,
                });
            }
        }
    }
    sides.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.side.cmp(&b.side)));
    churn.sort_by(|a, b| a.path.cmp(&b.path));
    import_deltas.sort_by(|a, b| a.path.cmp(&b.path));
    hunks.sort_by(|a, b| a.path.cmp(&b.path));
    AnalyzeFunctionsResult {
        sides,
        churn,
        import_deltas,
        hunks,
    }
}

fn to_analyzed(metrics: Vec<FunctionMetrics>) -> Vec<AnalyzedFunction> {
    metrics
        .into_iter()
        .map(|m| AnalyzedFunction {
            id: m.id,
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
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec(
                "src/foo.rs",
                Some("fn a() {}"),
                Some("fn a() { if true { 1; } }"),
            )],
        );
        assert_eq!(r.sides.len(), 2);
        let head = r.sides.iter().find(|s| s.side == "head").unwrap();
        assert_eq!(head.functions.len(), 1);
        assert!(head.functions[0].complexity >= 2.0);
    }

    #[test]
    fn an_added_file_has_only_a_head_side() {
        let r = analyze_files(
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec("src/new.py", None, Some("def f(x):\n    return x\n"))],
        );
        assert_eq!(r.sides.len(), 1);
        assert_eq!(r.sides[0].side, "head");
    }

    #[test]
    fn a_files_added_and_removed_imports_are_its_import_delta() {
        let r = analyze_files(
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec(
                "crates/oxplow-code-deps/src/lib.rs",
                Some("use std::fs;\nuse std::io;\nfn a() {}\n"),
                Some("use std::fs;\nuse oxplow_db::Database;\nfn a() {}\n"),
            )],
        );
        assert_eq!(r.import_deltas.len(), 1, "{r:?}");
        let delta = &r.import_deltas[0];
        let modules =
            |edges: &[ImportEdge]| edges.iter().map(|e| e.module.clone()).collect::<Vec<_>>();
        assert_eq!(modules(&delta.added), ["oxplow_db::Database"]);
        assert_eq!(modules(&delta.removed), ["std::io"]);
    }

    /// Unsupported languages still get (empty) sides, so the caller can
    /// see the file was looked at.
    #[test]
    fn unsupported_languages_get_empty_sides() {
        let r = analyze_files(
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec("README.md", Some("# old"), Some("# new"))],
        );
        assert_eq!(r.sides.len(), 2);
        assert!(r.sides[0].functions.is_empty());
    }

    #[test]
    fn a_modified_file_gets_its_functions_and_churn() {
        // The head changes alpha's body AND adds beta.
        let r = analyze_files(
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec(
                "src/x.rs",
                Some("fn alpha() -> i32 {\n    1\n}\n"),
                Some("fn alpha() -> i32 {\n    2\n}\n\nfn beta() -> i32 {\n    3\n}\n"),
            )],
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
            &oxplow_code_metrics::for_tests::shipped(),
            vec![spec("src/new.rs", None, Some("fn brand_new() {}\n"))],
        );
        assert!(r.sides[0].functions.iter().any(|f| f.name == "brand_new"));
        assert!(r.churn.is_empty(), "no base content, no before→after churn");
    }
}
