//! Tauri adapters for the code-quality command surface. The real
//! bodies live in `oxplow_rpc::commands::code_quality`; each command
//! here is a thin delegate so the headless daemon can dispatch the
//! same cores without `tauri`.

use oxplow_db::{CodeQualityFinding, CodeQualityScan};
use oxplow_git::co_change::FileSurprise;
use oxplow_tree_source::TreeVersion;

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::code_quality::{
    AnalyzeFileSpec, AnalyzeFunctionsResult, AnalyzedFileChurn, AnalyzedFileSide, AnalyzedFunction,
    AnalyzedFunctionChurn, FileFilterSpec, ImportDelta,
};

#[tauri::command]
#[specta::specta]
pub async fn list_code_quality_scans(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<CodeQualityScan>, IpcError> {
    oxplow_rpc::commands::code_quality::list_code_quality_scans(&state, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_code_quality_findings(
    state: tauri::State<'_, AppState>,
    scan_id: i64,
) -> Result<Vec<CodeQualityFinding>, IpcError> {
    oxplow_rpc::commands::code_quality::list_code_quality_findings(&state, scan_id).await
}

/// Run a fresh code-quality scan, persist findings, and return the
/// scan id. `tool` selects the analysis kind: `"metrics"` for
/// per-function complexity/length/parameters, `"duplication"` for
/// duplicate-block detection. `scope` is a free-form label
/// (typically `"workspace"` or `"diff"`).
#[tauri::command]
#[specta::specta]
pub async fn run_code_quality_scan(
    state: tauri::State<'_, AppState>,
    tool: String,
    scope: String,
    files: Option<Vec<String>>,
) -> Result<i64, IpcError> {
    oxplow_rpc::commands::code_quality::run_code_quality_scan(&state, tool, scope, files).await
}

/// Run a duplicate-block scan against `tree_version`, scoped by
/// `file_filter`. The corpus is the WHOLE tree at the requested
/// version — `file_filter` defines which files findings are
/// anchored to (the renderer's "side A"). A copy-paste from an
/// unchanged peer file surfaces because that peer is in the corpus
/// even though it's outside scope. Same-path matches (a file vs
/// itself) are dropped. Persists the scan row with the version +
/// filter columns so [`find_latest_done_scan`] can pick it up on
/// the next page load. Returns the scan id.
///
/// The renderer wires this to the "Scan now" button on the
/// duplication card. There is intentionally no auto-trigger:
/// scanning a commit's tree with libgit2 + tree-sitter is slow on a
/// large repo, so we keep it user-initiated until that becomes
/// interactive enough to make implicit.
#[tauri::command]
#[specta::specta]
pub async fn run_duplication_scan_at(
    state: tauri::State<'_, AppState>,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
    scope: String,
) -> Result<i64, IpcError> {
    oxplow_rpc::commands::code_quality::run_duplication_scan_at(
        &state,
        tree_version,
        file_filter,
        scope,
    )
    .await
}

/// Look up the most recent successful scan for `(tool, treeVersion,
/// fileFilter)`. The renderer uses this to decide whether to show
/// findings or a "Scan now" CTA.
#[tauri::command]
#[specta::specta]
pub async fn find_latest_code_quality_scan(
    state: tauri::State<'_, AppState>,
    tool: String,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
) -> Result<Option<CodeQualityScan>, IpcError> {
    oxplow_rpc::commands::code_quality::find_latest_code_quality_scan(
        &state,
        tool,
        tree_version,
        file_filter,
    )
    .await
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
#[tauri::command]
#[specta::specta]
pub async fn analyze_co_change_surprise(
    state: tauri::State<'_, AppState>,
    file_paths: Vec<String>,
) -> Result<Vec<FileSurprise>, IpcError> {
    oxplow_rpc::commands::code_quality::analyze_co_change_surprise(&state, file_paths).await
}

/// Compute per-function metadata for the Change Analysis dashboard,
/// for both sides of the diff. Pure in-process call: walks each
/// (path, content) pair through tree-sitter.
#[tauri::command]
#[specta::specta]
pub async fn analyze_functions_at_refs(
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    oxplow_rpc::commands::code_quality::analyze_functions(files).await
}
