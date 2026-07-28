//! Cores for the `code_quality` command module: scan listing/launch,
//! duplication scans at a tree version, co-change surprise, and the
//! per-function Change Analysis pipeline (metrics + churn + import
//! deltas).

use std::sync::Arc;

use oxplow_app::code_quality_runner::run_duplication_scan_scoped;
use oxplow_app::Services;
use oxplow_app::{BackgroundTaskKind, CodeQualityScanPhase, OxplowEvent, StartInput};
use oxplow_code_deps::{
    diff_edges, extract_imports, ImportEdge, ZoneRules, ZonedImportEdge, ZONE_EXTERNAL,
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

/// Abbreviate a ref for a human-facing progress label — a full sha is noise in
/// a status bar, so long refs show their first 7.
///
/// Truncates by CHARACTERS (tsk178). The original tested `len() > 12`, which
/// counts BYTES, then sliced `[..7]`, which demands a char boundary — so a
/// non-ASCII branch name passed the guard and panicked on the slice. That took
/// out the whole request (the daemon has no `CatchPanicLayer`, so no error
/// envelope) and stranded the scan row, created earlier, as permanently
/// "running".
fn short_ref(r#ref: &str) -> String {
    if r#ref.chars().count() > 12 {
        r#ref.chars().take(7).collect()
    } else {
        r#ref.to_string()
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
        TreeVersion::Ref { r#ref } => format!("Scanning duplicates @{}", short_ref(r#ref)),
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
            // Dual-write duplication facts beside the code_quality store (epic
            // tsk12, B). Resolve the built-in `oxplow.duplicate_lines` measure
            // once; build a fact per duplicate block alongside the finding rows,
            // then persist them under a single capture after the scan is done —
            // best-effort (never fails the scan).
            let dup_measure_id = svc
                .fact_store
                .get_measure("oxplow.duplicate_lines")
                .await
                .ok()
                .flatten()
                .map(|m| m.id);
            let mut dup_facts: Vec<oxplow_db::NewFact> = Vec::new();

            for f in findings {
                if let Some(mid) = dup_measure_id {
                    dup_facts.extend(duplication_fact(&f, mid));
                }
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

            // Always write the capture when the measure exists — an EMPTY one
            // is the "scanned, found nothing" record the currency/zero-fill
            // logic needs (tsk44), or the last non-empty scan's blocks stay
            // "current" forever after a refactor removes every duplicate.
            if dup_measure_id.is_some() {
                if let Err(e) =
                    write_duplication_facts(svc, dup_facts, &kind_tag, value_str.as_deref()).await
                {
                    tracing::warn!(error = %e, scan_id, "duplication: fact dual-write failed");
                }
            }
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

/// Map one duplication finding to an `oxplow.duplicate_lines` fact (epic tsk12,
/// B): value = the duplicated line count, subject = the block's `path:start-end`,
/// `path`/`line` the coordinate, the peer side carried verbatim in `detail`.
/// Only `duplicate-block` findings become facts (the sole kind the scan emits).
fn duplication_fact(
    f: &oxplow_app::code_quality_runner::CodeQualityFinding,
    measure_id: i64,
) -> Option<oxplow_db::NewFact> {
    if f.kind != "duplicate-block" {
        return None;
    }
    Some(oxplow_db::NewFact {
        subject_kind: Some("block".into()),
        subject_ref: Some(format!("{}:{}-{}", f.path, f.start_line, f.end_line)),
        path: Some(f.path.clone()),
        line: Some(f.start_line as i64),
        detail: f.extra_json.clone(),
        ..oxplow_db::NewFact::new(measure_id, f.metric_value)
    })
}

/// Persist duplication facts under one capture (epic tsk12, B). A duplication
/// scan has **no natural stream**, so the capture is stamped with the PRIMARY
/// stream (the scan runs over the primary worktree's tree); `basis_ref` + git
/// version carry the scanned tree version so the facts stay interpretable after
/// the fact. Best-effort — the caller logs on error and never fails the scan.
async fn write_duplication_facts(
    svc: &Services,
    facts: Vec<oxplow_db::NewFact>,
    kind_tag: &str,
    value: Option<&str>,
) -> Result<(), IpcError> {
    let streams = svc.streams.list_streams().await?;
    let Some(primary) = streams
        .iter()
        .find(|s| matches!(s.kind, oxplow_domain::StreamKind::Primary))
    else {
        return Ok(()); // no primary stream yet — nothing to attribute to
    };
    let basis = match value {
        Some(v) => format!("{kind_tag}:{v}"),
        None => kind_tag.to_string(),
    };
    let capture = oxplow_db::NewMetricCapture {
        basis_ref: Some(basis),
        closest_git_version: value.map(str::to_string),
        trigger: Some("manual".into()),
        ..oxplow_db::NewMetricCapture::done(primary.id.value(), "duplication", "duplication")
    };
    svc.fact_store.record_facts(capture, facts).await?;
    Ok(())
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
    svc: &Services,
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    // Zones are the project's own config (tsk251) — read them per
    // request so a `set_zones` edit shows up without a restart.
    let zones = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.zones.clone()
    };
    analyze_functions(files, &zones).await
}

/// `Services`-free implementation of [`analyze_functions_at_refs`].
/// The Tauri command for this analysis takes no state beyond the zone
/// table (the work is pure in-process tree-sitter), so its wrapper
/// delegates here while the dispatch registry routes through the
/// `svc`-taking core above.
pub async fn analyze_functions(
    files: Vec<AnalyzeFileSpec>,
    zones: &[oxplow_config::ZoneRuleConfig],
) -> Result<AnalyzeFunctionsResult, IpcError> {
    if files.is_empty() {
        return Ok(AnalyzeFunctionsResult {
            sides: Vec::new(),
            churn: Vec::new(),
            import_deltas: Vec::new(),
        });
    }
    let rules = ZoneRules::from_config(zones);
    let result = tokio::task::spawn_blocking(move || analyze_files(files, &rules))
        .await
        .map_err(|e| IpcError::internal(format!("analyze task: {e}")))?;
    Ok(result)
}

fn analyze_files(files: Vec<AnalyzeFileSpec>, zones: &ZoneRules) -> AnalyzeFunctionsResult {
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

    /// A project zone table for the import tests — the same shape a
    /// project writes into `.oxplow/project.yaml` (tsk251). Oxplow has
    /// no built-in table, so every zone assertion below is asserting
    /// against THESE rules, not against oxplow's opinion of the repo.
    fn zones() -> Vec<oxplow_config::ZoneRuleConfig> {
        [
            ("analysis", "crates/oxplow-code-deps/**"),
            ("store", "crates/oxplow-db/**"),
        ]
        .into_iter()
        .map(|(zone, pattern)| oxplow_config::ZoneRuleConfig {
            patterns: vec![pattern.to_string()],
            zone: zone.to_string(),
            color: None,
        })
        .collect()
    }

    #[tokio::test]
    async fn analyze_functions_returns_function_for_each_side() {
        let files = vec![AnalyzeFileSpec {
            path: "src/foo.rs".into(),
            base_content: Some("fn a() {}".into()),
            head_content: Some("fn a() { if true { 1; } }".into()),
        }];
        let result = analyze_functions(files, &zones()).await.unwrap();
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
        let result = analyze_functions(files, &zones()).await.unwrap();
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
        let result = analyze_functions(files, &zones()).await.unwrap();
        assert_eq!(result.import_deltas.len(), 1);
        let delta = &result.import_deltas[0];
        assert!(
            !delta.cross_zone_added.is_empty(),
            "expected cross-zone added; got delta={delta:?}"
        );
        let cz = &delta.cross_zone_added[0];
        assert_eq!(cz.from_zone, "analysis");
        assert_eq!(cz.to_zone.as_deref(), Some("store"));
    }

    /// The unconfigured project: no `zones:` block means no zone
    /// vocabulary, so nothing can be "cross-zone". The import delta is
    /// still reported — only the architectural read-out goes quiet.
    #[tokio::test]
    async fn without_a_zone_table_nothing_is_cross_zone() {
        let files = vec![AnalyzeFileSpec {
            path: "crates/oxplow-code-deps/src/lib.rs".into(),
            base_content: Some("use std::fs;\nfn a() {}\n".into()),
            head_content: Some("use std::fs;\nuse oxplow_db::Database;\nfn a() {}\n".into()),
        }];
        let result = analyze_functions(files, &[]).await.unwrap();
        let delta = &result.import_deltas[0];
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].from_zone, oxplow_code_deps::ZONE_OTHER);
        assert!(delta.cross_zone_added.is_empty());
    }

    #[tokio::test]
    async fn external_import_not_flagged_as_cross_zone() {
        let files = vec![AnalyzeFileSpec {
            path: "crates/oxplow-db/src/lib.rs".into(),
            base_content: Some("fn a() {}\n".into()),
            head_content: Some("use serde::Serialize;\nfn a() {}\n".into()),
        }];
        let result = analyze_functions(files, &zones()).await.unwrap();
        assert_eq!(result.import_deltas.len(), 1);
        let delta = &result.import_deltas[0];
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].to_zone.as_deref(), Some(ZONE_EXTERNAL));
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
        let result = analyze_functions(files, &zones()).await.unwrap();
        // We still emit empty sides so the caller can see "we looked".
        assert_eq!(result.sides.len(), 2);
        assert!(result.sides[0].functions.is_empty());
    }

    #[tokio::test]
    async fn duplication_scan_dual_writes_facts() {
        let (svc, dir) = crate::test_support::services();
        svc.streams.ensure_primary().await.unwrap();
        // Two files sharing a >=10-line identical block → a duplicate-block finding
        // (production DupOptions::default().n == 10).
        let body = "pub fn compute(input: &[i64]) -> i64 {\n\
        \x20   let mut total = 0;\n\
        \x20   for value in input {\n\
        \x20       if *value > 0 {\n\
        \x20           total += *value;\n\
        \x20       } else {\n\
        \x20           total -= *value;\n\
        \x20       }\n\
        \x20   }\n\
        \x20   total * 2 + 1\n\
        }\n";
        std::fs::write(dir.path().join("a.rs"), body).unwrap();
        std::fs::write(dir.path().join("b.rs"), body).unwrap();

        let scan_id = run_duplication_scan_at(
            &svc,
            TreeVersion::Disk,
            FileFilterSpec::All,
            "project".into(),
        )
        .await
        .unwrap();
        assert!(scan_id > 0);

        // The scan's duplicate blocks are mirrored as oxplow.duplicate_lines facts.
        let measure = svc
            .fact_store
            .get_measure("oxplow.duplicate_lines")
            .await
            .unwrap()
            .expect("built-in measure seeded");
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert!(!facts.is_empty(), "duplication facts written");
        assert!(
            facts.iter().all(|f| f.value >= 10.0),
            "value is the duplicated line count (>= min block size), got {:?}",
            facts.iter().map(|f| f.value).collect::<Vec<_>>()
        );
        assert!(facts
            .iter()
            .all(|f| f.subject_kind.as_deref() == Some("block")));
        assert!(
            facts.iter().all(|f| f.detail.is_some()),
            "peer side carried in detail"
        );
        // The capture is stamped with the primary stream + tree basis_ref.
        let cap = svc
            .fact_store
            .get_capture(facts[0].capture_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cap.producer, "duplication");
        assert!(cap.basis_ref.is_some());
    }

    #[tokio::test]
    async fn duplication_scan_survives_a_multibyte_ref() {
        // tsk178: the background-task label truncated a ref by BYTES after
        // testing its length in bytes, so a non-ASCII branch name panicked on a
        // mid-character slice. The scan row and Started event are emitted before
        // the label is built, so the panic also stranded the row as permanently
        // "running" — and with no CatchPanicLayer on the daemon, the request
        // died without an error envelope.
        //
        // 33 bytes, 11 chars: passes a `> 12` byte check, and byte 7 lands
        // mid-character.
        let (svc, _dir) = crate::test_support::services();
        svc.streams.ensure_primary().await.unwrap();
        let multibyte = "日本語ブランチ名テスト";
        assert_eq!(multibyte.len(), 33, "33 bytes");
        assert_eq!(multibyte.chars().count(), 11, "but only 11 chars");
        assert!(
            !multibyte.is_char_boundary(7),
            "byte 7 is mid-character — the old `&r#ref[..7]` panicked here"
        );

        // Short enough by CHARACTER count, so it is passed through whole.
        assert_eq!(short_ref(multibyte), multibyte);
        // A long ref still abbreviates, and on a char boundary.
        assert_eq!(short_ref("abcdef0123456789"), "abcdef0");
        // 7 CHARACTERS, not 7 bytes — which would have split the first one.
        assert_eq!(
            short_ref("日本語ブランチ名テストです工事中"),
            "日本語ブランチ"
        );
        assert_eq!(short_ref("main"), "main");

        // And the whole command path survives it rather than unwinding.
        let _ = run_duplication_scan_at(
            &svc,
            TreeVersion::Ref {
                r#ref: multibyte.into(),
            },
            FileFilterSpec::All,
            "project".into(),
        )
        .await;
    }

    #[tokio::test]
    async fn duplication_rescan_with_zero_hits_records_the_zero() {
        // tsk44 semantics: "scanned, found nothing" writes an EMPTY capture so
        // the currency logic drops the stale blocks — otherwise the metric
        // reports the pre-refactor duplication forever while the Code quality
        // panel shows clean.
        let (svc, dir) = crate::test_support::services();
        svc.streams.ensure_primary().await.unwrap();
        let body = "pub fn compute(input: &[i64]) -> i64 {\n\
        \x20   let mut total = 0;\n\
        \x20   for value in input {\n\
        \x20       if *value > 0 {\n\
        \x20           total += *value;\n\
        \x20       } else {\n\
        \x20           total -= *value;\n\
        \x20       }\n\
        \x20   }\n\
        \x20   total * 2 + 1\n\
        }\n";
        std::fs::write(dir.path().join("a.rs"), body).unwrap();
        std::fs::write(dir.path().join("b.rs"), body).unwrap();
        run_duplication_scan_at(
            &svc,
            TreeVersion::Disk,
            FileFilterSpec::All,
            "project".into(),
        )
        .await
        .unwrap();
        let rollup = svc
            .metric_engine
            .rollup("oxplow.duplicate_lines", "oxplow.package")
            .await
            .unwrap();
        assert!(
            !rollup.is_empty(),
            "duplicates present after the first scan"
        );

        // The refactor removes every duplicate; the rescan must clear the
        // metric's current state.
        std::fs::remove_file(dir.path().join("b.rs")).unwrap();
        run_duplication_scan_at(
            &svc,
            TreeVersion::Disk,
            FileFilterSpec::All,
            "project".into(),
        )
        .await
        .unwrap();
        let rollup = svc
            .metric_engine
            .rollup("oxplow.duplicate_lines", "oxplow.package")
            .await
            .unwrap();
        assert!(
            rollup.is_empty(),
            "a zero-hit rescan clears the current state, got {rollup:?}"
        );
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
