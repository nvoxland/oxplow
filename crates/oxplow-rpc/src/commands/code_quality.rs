//! Cores for the `code_quality` command module: scan listing/launch,
//! duplication scans at a tree version, co-change surprise, and the
//! per-function Change Analysis pipeline (metrics + churn + import
//! deltas).

use std::sync::Arc;

use oxplow_app::code_quality_runner::run_duplication_scan_scoped;
use oxplow_app::Services;
use oxplow_app::{BackgroundTaskKind, CodeQualityScanPhase, OxplowEvent, StartInput};
use oxplow_code_deps::{
    diff_edges, extract_imports, zone_for_resolved_edge, zone_for_unresolved_edge, ImportEdge,
    Zone, ZonedImportEdge,
};
use oxplow_code_metrics::{analyze_file, FunctionMetrics, Visibility};
use oxplow_db::{CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus};
use oxplow_git::co_change::{
    analyze_surprise, build_history, CoChangeOptions, FileSurprise, DEFAULT_DORMANT_DAYS,
};
use oxplow_tree_source::{
    AllFiles, DiskTreeSource, ExplicitPaths, FileFilter, GitTreeSource, TreeSource, TreeVersion,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

pub async fn list_code_quality_findings(
    svc: &Services,
    scan_id: i64,
) -> Result<Vec<CodeQualityFinding>, IpcError> {
    Ok(svc.code_quality_store.list_findings(scan_id).await?)
}

/// File filter the renderer can request: `all` (whole corpus) or an
/// explicit set of repo-relative paths. The serialized shape mirrors
/// the persisted `file_filter` column — callers pass `kind: "all"` or
/// `{ kind: "explicit", paths: [...] }`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FileFilterSpec {
    All,
    Explicit { paths: Vec<String> },
}

impl FileFilterSpec {
    fn fingerprint(&self) -> String {
        match self {
            FileFilterSpec::All => "all".into(),
            FileFilterSpec::Explicit { paths } => {
                use std::hash::{Hash, Hasher};
                let mut sorted: Vec<&String> = paths.iter().collect();
                sorted.sort();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for p in &sorted {
                    p.hash(&mut hasher);
                }
                format!("explicit:{:016x}", hasher.finish())
            }
        }
    }

    fn into_filter(self) -> Arc<dyn FileFilter> {
        match self {
            FileFilterSpec::All => Arc::new(AllFiles),
            FileFilterSpec::Explicit { paths } => Arc::new(ExplicitPaths::new(paths)),
        }
    }
}

/// Run a duplicate-block scan against `tree_version`, scoped by
/// `file_filter`. The corpus is the WHOLE tree at the requested
/// version — `file_filter` defines which files findings are
/// anchored to (the renderer's "side A"). A copy-paste from an
/// unchanged peer file surfaces because that peer is in the corpus
/// even though it's outside scope. Same-path matches (a file vs
/// itself) are dropped. Persists the scan row with the version +
/// filter columns so [`find_latest_code_quality_scan`] can pick it
/// up on the next page load. Returns the scan id.
///
/// The renderer wires this to the "Scan now" button on the
/// duplication card. There is intentionally no auto-trigger:
/// scanning a commit's tree with libgit2 + tree-sitter is slow on a
/// large repo, so we keep it user-initiated until that becomes
/// interactive enough to make implicit.
pub async fn run_duplication_scan_at(
    svc: &Services,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
    scope: String,
) -> Result<i64, IpcError> {
    let project = svc.layout.project_dir.clone();
    let kind_tag = tree_version.kind_tag().to_string();
    let value_str = tree_version.value().map(str::to_string);
    let filter_fp = file_filter.fingerprint();
    let filter = file_filter.into_filter();

    let source: Arc<dyn TreeSource> = match &tree_version {
        TreeVersion::Disk => Arc::new(DiskTreeSource::new(project.clone())),
        TreeVersion::Ref { r#ref } => Arc::new(GitTreeSource::new(project.clone(), r#ref.clone())),
        TreeVersion::Snapshot { .. } => {
            return Err(IpcError::invalid(
                "snapshot tree version is not yet implemented",
            ));
        }
    };

    let scan_id = svc
        .code_quality_store
        .create_scan_with(
            "duplication",
            &scope,
            &kind_tag,
            value_str.as_deref(),
            &filter_fp,
        )
        .await?;
    svc.events.emit(OxplowEvent::CodeQualityScanned {
        stream_id: None,
        scan_id,
        tool: "duplication".into(),
        scope: scope.clone(),
        phase: CodeQualityScanPhase::Started,
    });
    // Surface to the StatusBar's BackgroundTaskIndicator so the user
    // gets the standard "running" affordance while the scan runs.
    let bg_label = match &tree_version {
        TreeVersion::Disk => "Scanning duplicates (working tree)".to_string(),
        TreeVersion::Ref { r#ref } => {
            let short = if r#ref.len() > 12 {
                &r#ref[..7]
            } else {
                r#ref.as_str()
            };
            format!("Scanning duplicates @{short}")
        }
        TreeVersion::Snapshot { id } => format!("Scanning duplicates @snapshot {id}"),
    };
    let bg_task = svc.background_tasks.start(StartInput {
        kind: BackgroundTaskKind::CodeQuality,
        label: bg_label,
        detail: Some(format!("scope: {scope}")),
        progress: None,
    });

    let workspace_filter = {
        let cfg = svc.config.read();
        cfg.as_ref()
            .map(|c| {
                oxplow_fs_watch::WorkspaceFilter::for_project(
                    &svc.layout.project_dir,
                    &c.generated.exclude,
                    &c.generated.include,
                )
            })
            .unwrap_or_default()
    };
    match run_duplication_scan_scoped(source, filter, workspace_filter, None, None).await {
        Ok(findings) => {
            for f in findings {
                svc.code_quality_store
                    .append_finding(
                        scan_id,
                        oxplow_db::CodeQualityFinding {
                            id: 0,
                            scan_id,
                            path: f.path,
                            start_line: f.start_line as i32,
                            end_line: f.end_line as i32,
                            kind: f.kind,
                            metric_value: f.metric_value,
                            extra_json: f.extra_json,
                        },
                    )
                    .await?;
            }
            svc.code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Done, None)
                .await?;
            svc.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: "duplication".into(),
                scope,
                phase: CodeQualityScanPhase::Completed,
            });
            svc.background_tasks.complete(&bg_task.id, None);
            Ok(scan_id)
        }
        Err(e) => {
            svc.code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.to_string()))
                .await?;
            svc.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: "duplication".into(),
                scope,
                phase: CodeQualityScanPhase::Failed,
            });
            svc.background_tasks.fail(&bg_task.id, e.to_string(), None);
            Err(IpcError::internal(e.to_string()))
        }
    }
}

/// Look up the most recent successful scan for `(tool, treeVersion,
/// fileFilter)`. The renderer uses this to decide whether to show
/// findings or a "Scan now" CTA.
pub async fn find_latest_code_quality_scan(
    svc: &Services,
    tool: String,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
) -> Result<Option<CodeQualityScan>, IpcError> {
    let kind_tag = tree_version.kind_tag().to_string();
    let value_str = tree_version.value().map(str::to_string);
    let filter_fp = file_filter.fingerprint();
    Ok(svc
        .code_quality_store
        .find_latest_done_scan(&tool, &kind_tag, value_str.as_deref(), &filter_fp)
        .await?)
}

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

/// Classify each path against the project's commit-history co-change
/// patterns. Returns one [`FileSurprise`] per input path explaining
/// whether the touch is `Normal`, has missing-usual-co-changers, or
/// the file is `Dormant`.
///
/// History is rebuilt on every call — fast enough for diff-time
/// invocations (≤ 5000 commits, sub-second on oxplow-scale repos).
/// Caching the [`CoChangeHistory`] per project is a runtime concern
/// the caller can layer on top later.
pub async fn analyze_co_change_surprise(
    svc: &Services,
    file_paths: Vec<String>,
) -> Result<Vec<FileSurprise>, IpcError> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }
    let project = svc.layout.project_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let history = build_history(&project, CoChangeOptions::default());
        analyze_surprise(&history, &file_paths, DEFAULT_DORMANT_DAYS)
    })
    .await
    .map_err(|e| IpcError::internal(format!("co-change task: {e}")))?;
    Ok(result)
}

/// Compute per-function metadata for the Change Analysis dashboard,
/// for both sides of the diff. Pure in-process call: walks each
/// (path, content) pair through tree-sitter.
pub async fn analyze_functions_at_refs(
    _svc: &Services,
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    analyze_functions(files).await
}

/// `Services`-free implementation of [`analyze_functions_at_refs`].
/// The Tauri command for this analysis takes no state (the work is
/// pure in-process tree-sitter), so its wrapper delegates here while
/// the dispatch registry routes through the `_svc`-taking core above.
pub async fn analyze_functions(
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    if files.is_empty() {
        return Ok(AnalyzeFunctionsResult {
            sides: Vec::new(),
            churn: Vec::new(),
            import_deltas: Vec::new(),
        });
    }
    let result = tokio::task::spawn_blocking(move || analyze_files(files))
        .await
        .map_err(|e| IpcError::internal(format!("analyze task: {e}")))?;
    Ok(result)
}

fn analyze_files(files: Vec<AnalyzeFileSpec>) -> AnalyzeFunctionsResult {
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
            let fc = crate::commands::churn::compute_file_churn(
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
                let added: Vec<ZonedImportEdge> = added_raw.into_iter().map(zone_edge).collect();
                let removed: Vec<ZonedImportEdge> =
                    removed_raw.into_iter().map(zone_edge).collect();
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
///   `zone_for_crate_name`; if it returns `None` we mark the target
///   as `External` (a real crate name we don't host).
/// - TS/JS `import "./foo"` / `"../bar"`: relative paths join with
///   the importer's directory. The joined path goes through the
///   path zone classifier. Non-relative ("react", "@scope/pkg")
///   marks as `External`.
/// - Everything else: unresolved (to_zone = None), so cross-zone
///   logic ignores it. Better to underflag than overflag.
fn zone_edge(edge: ImportEdge) -> ZonedImportEdge {
    if let Some(target) = resolve_target(&edge) {
        match target {
            ResolveResult::RepoPath(path) => zone_for_resolved_edge(edge, &path),
            ResolveResult::External => {
                // Build a synthetic edge whose to_zone is External.
                let from_zone = oxplow_code_deps::classify_zone(&edge.from_path);
                ZonedImportEdge {
                    edge,
                    from_zone,
                    to_zone: Some(Zone::External),
                }
            }
        }
    } else {
        zone_for_unresolved_edge(edge)
    }
}

enum ResolveResult {
    /// In-repo file path (or the synthetic `crates/<name>/src/lib.rs`
    /// stand-in for workspace crate references).
    RepoPath(String),
    /// Definitely not in this repo (system lib, npm package, etc.).
    External,
}

fn resolve_target(edge: &ImportEdge) -> Option<ResolveResult> {
    use oxplow_code_deps::ImportKind;
    match edge.kind {
        ImportKind::Use => resolve_rust(edge),
        ImportKind::Import => resolve_ts_like(edge),
        ImportKind::PyImport
        | ImportKind::GoImport
        | ImportKind::JavaImport
        | ImportKind::Include
        | ImportKind::Using
        | ImportKind::CljRequire => None,
    }
}

fn resolve_rust(edge: &ImportEdge) -> Option<ResolveResult> {
    let first = edge.module.split("::").next().unwrap_or("");
    if first.is_empty() {
        return None;
    }
    if matches!(first, "crate" | "self" | "super") {
        // Resolves back inside the importer's own crate — same zone
        // by construction.
        return Some(ResolveResult::RepoPath(edge.from_path.clone()));
    }
    if let Some(zone) = oxplow_code_deps::zone_for_crate_name(first) {
        // Synthesize a path that classifies to that zone — we don't
        // need the exact file, just the zone.
        let _ = zone;
        return Some(ResolveResult::RepoPath(format!(
            "crates/{}/src/lib.rs",
            first.replace('_', "-")
        )));
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

    #[tokio::test]
    async fn analyze_functions_returns_function_for_each_side() {
        let files = vec![AnalyzeFileSpec {
            path: "src/foo.rs".into(),
            base_content: Some("fn a() {}".into()),
            head_content: Some("fn a() { if true { 1; } }".into()),
        }];
        let result = analyze_functions(files).await.unwrap();
        assert_eq!(result.sides.len(), 2);
        let head = result.sides.iter().find(|s| s.side == "head").unwrap();
        assert_eq!(head.functions.len(), 1);
        assert!(head.functions[0].complexity >= 2.0);
    }

    #[tokio::test]
    async fn analyze_functions_handles_added_file() {
        let files = vec![AnalyzeFileSpec {
            path: "src/new.py".into(),
            base_content: None,
            head_content: Some("def f(x):\n    return x\n".into()),
        }];
        let result = analyze_functions(files).await.unwrap();
        assert_eq!(result.sides.len(), 1);
        assert_eq!(result.sides[0].side, "head");
    }

    #[tokio::test]
    async fn cross_zone_import_added_surfaces() {
        // UI file gains a Rust-style import of `oxplow_db` — but
        // this is a Rust source path, so use a Rust importer. Use
        // an analysis-zone file that adds an import of oxplow_db.
        let files = vec![AnalyzeFileSpec {
            path: "crates/oxplow-code-deps/src/lib.rs".into(),
            base_content: Some("use std::fs;\nfn a() {}\n".into()),
            head_content: Some("use std::fs;\nuse oxplow_db::Database;\nfn a() {}\n".into()),
        }];
        let result = analyze_functions(files).await.unwrap();
        assert_eq!(result.import_deltas.len(), 1);
        let delta = &result.import_deltas[0];
        assert!(
            !delta.cross_zone_added.is_empty(),
            "expected cross-zone added; got delta={delta:?}"
        );
        let cz = &delta.cross_zone_added[0];
        assert_eq!(cz.from_zone, Zone::Analysis);
        assert_eq!(cz.to_zone, Some(Zone::Store));
    }

    #[tokio::test]
    async fn external_import_not_flagged_as_cross_zone() {
        let files = vec![AnalyzeFileSpec {
            path: "crates/oxplow-db/src/lib.rs".into(),
            base_content: Some("fn a() {}\n".into()),
            head_content: Some("use serde::Serialize;\nfn a() {}\n".into()),
        }];
        let result = analyze_functions(files).await.unwrap();
        assert_eq!(result.import_deltas.len(), 1);
        let delta = &result.import_deltas[0];
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].to_zone, Some(Zone::External));
        // External targets are deliberately NOT cross-zone — a
        // store crate pulling in serde is not a layer violation.
        assert!(
            delta.cross_zone_added.is_empty(),
            "External targets must not surface as cross-zone; got {:?}",
            delta.cross_zone_added
        );
    }

    #[tokio::test]
    async fn analyze_functions_skips_unsupported_languages() {
        let files = vec![AnalyzeFileSpec {
            path: "README.md".into(),
            base_content: Some("# old".into()),
            head_content: Some("# new".into()),
        }];
        let result = analyze_functions(files).await.unwrap();
        // We still emit empty sides so the caller can see "we looked".
        assert_eq!(result.sides.len(), 2);
        assert!(result.sides[0].functions.is_empty());
    }

    #[tokio::test]
    async fn analyze_functions_at_refs_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "analyze_functions_at_refs",
            serde_json::json!({
                "files": [{
                    "path": "src/foo.rs",
                    "base_content": "fn a() {}",
                    "head_content": "fn a() { if true { 1; } }",
                }]
            }),
            &svc,
        )
        .await
        .unwrap();
        let sides = out.get("sides").and_then(|s| s.as_array()).unwrap();
        assert_eq!(sides.len(), 2);
    }
}
