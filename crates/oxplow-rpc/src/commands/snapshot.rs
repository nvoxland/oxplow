//! Cores for the `snapshot` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use specta::Type;

use oxplow_app::blob_store::BlobStore;
use oxplow_app::Services;
use oxplow_db::{FileSnapshot, Snapshot, SnapshotChangeEntry, SnapshotStats};
use oxplow_domain::StreamId;
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;

/// Build a `WorkspaceFilter` from the project's currently-live
/// `generated` config. Used by the snapshot-list IPCs so that paths
/// matching the user's current ignore list are stripped from the
/// returned list — even if those paths were captured under an older
/// config (or before they were marked generated). The capture
/// pipeline already filters going forward via this same struct; this
/// is the read-side complement.
fn current_filter(svc: &Services) -> WorkspaceFilter {
    let cfg = svc.config.read();
    cfg.as_ref()
        .map(|c| {
            WorkspaceFilter::for_project(
                &svc.layout.project_dir,
                &c.generated.exclude,
                &c.generated.include,
            )
        })
        .unwrap_or_default()
}

pub async fn list_snapshots(svc: &Services, path: String) -> Result<Vec<FileSnapshot>, IpcError> {
    // Whole-path filter: if the queried path itself is currently
    // marked generated, return an empty history rather than the
    // pre-config captures. The UI shouldn't surface a "history" view
    // for a path the user has declared they don't care about.
    let filter = current_filter(svc);
    if filter.ignore(Path::new(&path), false) {
        return Ok(Vec::new());
    }
    Ok(svc.snapshot_store.list_for_path(&path).await?)
}

pub async fn list_file_snapshots_for_stream(
    svc: &Services,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<FileSnapshot>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_for_stream(stream_id, limit.unwrap_or(200))
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

/// `snapshot` rows for a stream — one entry per `request_snapshot()`
/// call that captured anything. Newest first.
pub async fn list_snapshots_for_stream(
    svc: &Services,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<Snapshot>, IpcError> {
    Ok(svc
        .snapshot_store
        .list_snapshots_for_stream(stream_id, limit.unwrap_or(200))
        .await?)
}

/// Created/modified/deleted counts for a snapshot. Powers the Local
/// History dashboard's per-snapshot stats column.
pub async fn get_snapshot_stats(
    svc: &Services,
    snapshot_id: i64,
) -> Result<SnapshotStats, IpcError> {
    Ok(svc.snapshot_store.stats_for_snapshot(snapshot_id).await?)
}

/// Per-file change entries for one snapshot, in the shape the
/// renderer's `useSnapshotChangeAnalysis` hook expects so it can
/// feed the same SummaryCard / ChangeAnalysisPanel components the
/// Git pages use.
pub async fn list_snapshot_change_entries(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Vec<SnapshotChangeEntry>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_changes_for_snapshot(snapshot_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

/// Read a `file_snapshot` row's blob content as a UTF-8 string.
/// Returns `None` when:
/// - the row id doesn't exist,
/// - the row has no blob hash (deletion row or oversize-tracked),
/// - the blob has been pruned from disk.
///
/// Binary bytes pass through as UTF-8 lossy — the renderer's diff /
/// function-analysis pipeline treats the result as text either way.
pub async fn read_snapshot_file_content(
    svc: &Services,
    file_snapshot_id: i64,
) -> Result<Option<String>, IpcError> {
    let Some(snap) = svc.snapshot_store.get(file_snapshot_id).await? else {
        return Ok(None);
    };
    let Some(hash) = snap.blob_hash.clone() else {
        return Ok(None);
    };
    let blobs = svc.blobs.clone();
    let project_dir = svc.layout.project_dir.clone();
    let storage = snap.storage;
    let bytes = tokio::task::spawn_blocking(move || {
        oxplow_app::snapshot_content::read_snapshot_content(storage, &hash, &project_dir, &blobs)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    match bytes {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(_) => Ok(None),
    }
}

/// Total on-disk size of every blob in the content-addressed store.
/// Used by the Local History dashboard's Storage card.
pub async fn get_blob_storage_bytes(svc: &Services) -> Result<i64, IpcError> {
    let blobs = svc.blobs.clone();
    let total = tokio::task::spawn_blocking(move || blobs.total_bytes())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(total as i64)
}

/// For each snapshot id in the input list, the wiki slugs whose
/// body changed in that snapshot. Drives the Local History
/// dashboard's wiki badges. Cheaper than fetching the full
/// `file_snapshot` rows per snapshot.
pub async fn list_wiki_slugs_for_snapshots(
    svc: &Services,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<(i64, String)>, IpcError> {
    Ok(svc
        .snapshot_store
        .list_wiki_slugs_for_snapshots(snapshot_ids)
        .await?)
}

/// Every `file_snapshot` row captured under a single parent
/// snapshot id (i.e. one batch of `request_snapshot()`).
pub async fn list_files_for_snapshot(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Vec<FileSnapshot>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc
        .snapshot_store
        .list_files_for_snapshot(snapshot_id)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|r| !filter.ignore(Path::new(&r.path), false))
        .collect())
}

pub async fn get_snapshot(svc: &Services, id: i64) -> Result<Option<FileSnapshot>, IpcError> {
    Ok(svc.snapshot_store.get(id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotPairDiff {
    pub before: Option<FileSnapshot>,
    pub after: Option<FileSnapshot>,
    /// True when the two captures hash differently (i.e. content
    /// changed between them). Always false when either side is None.
    pub changed: bool,
}

/// Compare two captures of the same path. The renderer surfaces this
/// in the snapshots panel as "what changed between then and now".
pub async fn get_snapshot_pair_diff(
    svc: &Services,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> Result<SnapshotPairDiff, IpcError> {
    let before = match before_id {
        Some(id) => svc.snapshot_store.get(id).await?,
        None => None,
    };
    let after = match after_id {
        Some(id) => svc.snapshot_store.get(id).await?,
        None => None,
    };
    let changed = match (&before, &after) {
        (Some(b), Some(a)) => b.blob_hash != a.blob_hash,
        _ => false,
    };
    Ok(SnapshotPairDiff {
        before,
        after,
        changed,
    })
}

/// One endpoint of a diff: a captured local-history snapshot, a git
/// commit (any revspec libgit2 resolves), or the live working tree
/// (reserved for an in-progress effort's open end).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffEndpoint {
    Snapshot { snapshot_id: i64 },
    Commit { sha: String },
    Working,
}

/// One changed path between two [`DiffEndpoint`]s. `status` is
/// `"added" | "modified" | "deleted"`, matching the renderer's
/// `BranchChangeEntry`. `additions`/`deletions` are per-file line
/// counts (via `similar`), `0` only for binary, oversize, or otherwise
/// unreadable content.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

fn change_status_str(s: oxplow_domain::ChangeStatus) -> &'static str {
    match s {
        oxplow_domain::ChangeStatus::Added => "added",
        oxplow_domain::ChangeStatus::Modified => "modified",
        oxplow_domain::ChangeStatus::Deleted => "deleted",
    }
}

/// Diff two endpoints. `start = None` diffs `end` against the empty
/// tree (everything added).
///
/// The unified resolver builds a `path -> content cell` tree for each
/// endpoint, then compares. Same-identity-space pairs (snapshot↔snapshot,
/// commit↔commit, commit↔working, …) compare raw — no byte reads.
/// Cross-space pairs (snapshot↔commit, snapshot↔working) **normalize**
/// the snapshot side into git-oid space (read each oxplow-stored blob's
/// bytes, recompute its git blob oid) so it compares like-for-like
/// against the git tree. Oversize / pruned content keeps a best-effort
/// opaque identity. `additions`/`deletions` are per-file line counts via
/// `similar`, computed for the changed set only.
pub async fn diff_endpoints(
    svc: &Services,
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
) -> Result<Vec<DiffEntry>, IpcError> {
    // Snapshot trees come off the DB (async); prefetch them, then do the
    // git / fs / hashing / diff work on the blocking pool.
    let start_snap = match &start {
        Some(DiffEndpoint::Snapshot { snapshot_id }) => {
            Some(svc.snapshot_store.tree_at(*snapshot_id).await?)
        }
        _ => None,
    };
    let end_snap = match &end {
        DiffEndpoint::Snapshot { snapshot_id } => {
            Some(svc.snapshot_store.tree_at(*snapshot_id).await?)
        }
        _ => None,
    };
    let project_dir = svc.layout.project_dir.clone();
    let blobs = svc.blobs.clone();
    let filter = current_filter(svc);
    tokio::task::spawn_blocking(move || {
        compute_diff(
            start,
            end,
            start_snap,
            end_snap,
            &project_dir,
            &blobs,
            &filter,
        )
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(IpcError::internal)
}

/// Read the UTF-8 (lossy) content of each `path` as of `endpoint`.
/// Returns a vec aligned to `paths`: `None` for a path absent at the
/// endpoint, or binary / oversize / pruned content. Builds the
/// endpoint's content tree once, then reads each requested path — the
/// diff view's function-level analysis calls this twice (base + head)
/// with the changed-file set, mirroring the snapshot/commit branches'
/// per-file content reads.
pub async fn read_endpoint_files_content(
    svc: &Services,
    endpoint: DiffEndpoint,
    paths: Vec<String>,
) -> Result<Vec<Option<String>>, IpcError> {
    let snap = match &endpoint {
        DiffEndpoint::Snapshot { snapshot_id } => {
            Some(svc.snapshot_store.tree_at(*snapshot_id).await?)
        }
        _ => None,
    };
    let project_dir = svc.layout.project_dir.clone();
    let blobs = svc.blobs.clone();
    let filter = current_filter(svc);
    tokio::task::spawn_blocking(move || -> Result<Vec<Option<String>>, String> {
        let (cells, _space) = cells_for_endpoint(&endpoint, snap, &project_dir, &filter)?;
        Ok(paths
            .into_iter()
            .map(|p| {
                cells
                    .get(&p)
                    .and_then(|cell| read_cell(cell, &blobs, &project_dir))
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .collect())
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(IpcError::internal)
}

/// The identity space a tree's `Cell`s compare in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    /// xxh3 / git-oid / oversize-sentinel as stored by the snapshot store.
    Snapshot,
    /// git blob oids (commit trees, and the working tree we build in this
    /// space).
    Git,
}

/// How to fetch a path's raw bytes for line counting.
#[derive(Clone)]
enum ContentSource {
    /// `id` is an xxh3 hash → oxplow blob store.
    Oxplow,
    /// `id` is a git blob oid → git odb.
    Git,
    /// live file on disk.
    Working(PathBuf),
    /// oversize / pruned — no readable bytes; identity is opaque.
    Unreadable,
}

/// A path's content within one endpoint's tree.
#[derive(Clone)]
struct Cell {
    /// Diff-comparison identity, in the cell's native [`Space`].
    id: String,
    source: ContentSource,
}

fn compute_diff(
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
    start_snap: Option<BTreeMap<String, String>>,
    end_snap: Option<BTreeMap<String, String>>,
    project_dir: &Path,
    blobs: &BlobStore,
    filter: &WorkspaceFilter,
) -> Result<Vec<DiffEntry>, String> {
    let (after_cells, after_space) = cells_for_endpoint(&end, end_snap, project_dir, filter)?;
    let (before_cells, before_space) = match &start {
        Some(ep) => {
            let (cells, space) = cells_for_endpoint(ep, start_snap, project_dir, filter)?;
            (cells, Some(space))
        }
        None => (BTreeMap::new(), None),
    };

    // Cross-space pairs normalize into git-oid space; same-space pairs
    // (incl. None start) compare raw identities with no byte reads.
    let normalize = before_space.is_some_and(|bs| bs != after_space);
    let before_ids: BTreeMap<String, String> = before_cells
        .iter()
        .map(|(p, c)| (p.clone(), compare_id(c, normalize, blobs)))
        .collect();
    let after_ids: BTreeMap<String, String> = after_cells
        .iter()
        .map(|(p, c)| (p.clone(), compare_id(c, normalize, blobs)))
        .collect();
    let changes = oxplow_domain::diff_trees(&before_ids, &after_ids);

    Ok(changes
        .into_iter()
        .map(|c| {
            let base = before_cells
                .get(&c.path)
                .and_then(|cell| read_cell(cell, blobs, project_dir));
            let head = after_cells
                .get(&c.path)
                .and_then(|cell| read_cell(cell, blobs, project_dir));
            let (additions, deletions) = count_lines(base.as_deref(), head.as_deref());
            DiffEntry {
                path: c.path,
                status: change_status_str(c.status).to_string(),
                additions,
                deletions,
            }
        })
        .collect())
}

/// Build one endpoint's `path -> Cell` tree + its identity space.
fn cells_for_endpoint(
    ep: &DiffEndpoint,
    snap_tree: Option<BTreeMap<String, String>>,
    project_dir: &Path,
    filter: &WorkspaceFilter,
) -> Result<(BTreeMap<String, Cell>, Space), String> {
    match ep {
        DiffEndpoint::Snapshot { .. } => {
            let cells = snap_tree
                .unwrap_or_default()
                .into_iter()
                .map(|(path, id)| (path, classify_snapshot_cell(id)))
                .collect();
            Ok((cells, Space::Snapshot))
        }
        DiffEndpoint::Commit { sha } => {
            let tree = oxplow_git::tree_at_commit(project_dir, sha).map_err(|e| e.to_string())?;
            let cells = tree
                .into_iter()
                .map(|(path, oid)| {
                    (
                        path,
                        Cell {
                            id: oid,
                            source: ContentSource::Git,
                        },
                    )
                })
                .collect();
            Ok((cells, Space::Git))
        }
        DiffEndpoint::Working => Ok((working_cells(project_dir, filter), Space::Git)),
    }
}

/// Infer a snapshot row's content source from its `tree_at` identity:
/// 40-hex git oid, `oversize:…` sentinel, else a 32-hex xxh3 hash.
fn classify_snapshot_cell(id: String) -> Cell {
    let source = if id.starts_with("oversize:") {
        ContentSource::Unreadable
    } else if id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        ContentSource::Git
    } else {
        ContentSource::Oxplow
    };
    Cell { id, source }
}

/// Build the live working tree in git-oid space, honouring the
/// generated-file filter (the same walk the capture sweep uses). Clean
/// tracked files reuse their HEAD blob oid (no read); dirty / untracked
/// files are hashed from disk.
fn working_cells(project_dir: &Path, filter: &WorkspaceFilter) -> BTreeMap<String, Cell> {
    let clean = oxplow_git::clean_head_blob_oids(project_dir);
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(project_dir)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let rel = e.path().strip_prefix(project_dir).unwrap_or(e.path());
            !filter.ignore(rel, e.file_type().is_dir())
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(project_dir) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let abs = entry.path().to_path_buf();
        let id = match clean.get(&rel_str) {
            Some(oid) => oid.clone(),
            None => match std::fs::read(&abs)
                .ok()
                .as_deref()
                .and_then(oxplow_git::git_blob_oid)
            {
                Some(oid) => oid,
                None => continue,
            },
        };
        out.insert(
            rel_str,
            Cell {
                id,
                source: ContentSource::Working(abs),
            },
        );
    }
    out
}

/// The comparison identity for a cell. Raw when same-space; normalized
/// into git-oid space when crossing spaces (only an oxplow xxh3 cell
/// needs a byte read + rehash — git/working ids are already oids, an
/// oversize sentinel stays opaque). A failed read falls back to the raw
/// id (best-effort: it simply won't match, so the file reads as changed).
fn compare_id(cell: &Cell, normalize_to_git: bool, blobs: &BlobStore) -> String {
    if !normalize_to_git {
        return cell.id.clone();
    }
    match &cell.source {
        ContentSource::Oxplow => blobs
            .read(&cell.id)
            .ok()
            .as_deref()
            .and_then(oxplow_git::git_blob_oid)
            .unwrap_or_else(|| cell.id.clone()),
        _ => cell.id.clone(),
    }
}

/// Raw bytes for a cell, for line counting. `None` for oversize / pruned
/// content or a failed read.
fn read_cell(cell: &Cell, blobs: &BlobStore, project_dir: &Path) -> Option<Vec<u8>> {
    match &cell.source {
        ContentSource::Oxplow => blobs.read(&cell.id).ok(),
        ContentSource::Git => oxplow_git::read_blob(project_dir, &cell.id),
        ContentSource::Working(path) => std::fs::read(path).ok(),
        ContentSource::Unreadable => None,
    }
}

/// Added / deleted line counts between two blobs via `similar`. A
/// missing side is the empty file (added → all of head; deleted → all
/// of base). Binary content (NUL byte) yields `(0, 0)`.
fn count_lines(base: Option<&[u8]>, head: Option<&[u8]>) -> (u32, u32) {
    let is_binary = |b: &&[u8]| b.contains(&0);
    if base.filter(is_binary).is_some() || head.filter(is_binary).is_some() {
        return (0, 0);
    }
    let base_s = base
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let head_s = head
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let diff = TextDiff::from_lines(base_s.as_str(), head_s.as_str());
    let mut additions = 0u32;
    let mut deletions = 0u32;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    (additions, deletions)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    pub hash: String,
    pub mtime_ms: i64,
    pub size: i64,
    /// "present" for normal captures, "oversize" for files that
    /// exceeded the configured cap (no blob written).
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotFileRow {
    pub entry: SnapshotEntry,
    /// "created" when this is the first capture of `path`,
    /// "updated" when the prior capture had a different blob,
    /// "deleted" when the current capture has no blob (file gone).
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Default)]
pub struct SnapshotSummaryCounts {
    pub created: i64,
    pub updated: i64,
    pub deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub snapshot: FileSnapshot,
    pub previous_snapshot_id: Option<String>,
    pub files: std::collections::HashMap<String, SnapshotFileRow>,
    pub counts: SnapshotSummaryCounts,
}

/// Build a per-snapshot summary: the FileSnapshot row, the id of the
/// prior capture of the same path (if any), and a one-row diff
/// describing how the captured file relates to its predecessor
/// (created / updated / deleted). The renderer's local-history pane
/// keys off this shape.
pub async fn get_snapshot_summary(
    svc: &Services,
    snapshot_id: i64,
) -> Result<Option<SnapshotSummary>, IpcError> {
    let Some(snap) = svc.snapshot_store.get(snapshot_id).await? else {
        return Ok(None);
    };
    // Order is DESC by captured_at; find the row immediately after
    // ours (i.e. older). Equal-timestamp ties fall back to id order
    // implicitly via SQLite's row order.
    let history = svc.snapshot_store.list_for_path(&snap.path).await?;
    let mut prev: Option<&FileSnapshot> = None;
    let mut found_self = false;
    for row in &history {
        if found_self {
            prev = Some(row);
            break;
        }
        if row.id == snap.id {
            found_self = true;
        }
    }
    let kind = match (&snap.blob_hash, prev.and_then(|p| p.blob_hash.clone())) {
        (None, _) => "deleted",
        (Some(_), None) => "created",
        (Some(cur), Some(prev_hash)) if *cur == prev_hash => "updated",
        (Some(_), Some(_)) => "updated",
    };
    let state_label = if snap.storage.is_oversize() {
        "oversize"
    } else {
        "present"
    };
    let entry = SnapshotEntry {
        hash: snap.blob_hash.clone().unwrap_or_default(),
        mtime_ms: 0,
        size: snap.size_bytes,
        state: state_label.to_string(),
    };
    let mut files = std::collections::HashMap::new();
    files.insert(
        snap.path.clone(),
        SnapshotFileRow {
            entry,
            kind: kind.to_string(),
        },
    );
    let counts = SnapshotSummaryCounts {
        created: if kind == "created" { 1 } else { 0 },
        updated: if kind == "updated" { 1 } else { 0 },
        deleted: if kind == "deleted" { 1 } else { 0 },
    };
    Ok(Some(SnapshotSummary {
        snapshot: snap,
        previous_snapshot_id: prev.map(|p| p.id.to_string()),
        files,
        counts,
    }))
}

/// Restore a file's contents from a snapshot. Reads the bytes from
/// the content-addressed blob store using the snapshot's `blob_hash`
/// and writes them back to the snapshot's path inside the workspace.
/// Errors with NOT_FOUND if the snapshot row is gone or its blob
/// was pruned.
pub async fn restore_file_from_snapshot(svc: &Services, snapshot_id: i64) -> Result<(), IpcError> {
    let snap = svc
        .snapshot_store
        .get(snapshot_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    let hash = snap
        .blob_hash
        .clone()
        .ok_or_else(|| IpcError::invalid("snapshot has no blob (oversize or deleted)"))?;
    // Route through the read seam so a git-backed row recovers its bytes
    // from the git odb instead of the (absent) blob store.
    let bytes = oxplow_app::snapshot_content::read_snapshot_content(
        snap.storage,
        &hash,
        &svc.layout.project_dir,
        &svc.blobs,
    )
    .map_err(|e| IpcError::internal(e.to_string()))?;
    let target = svc.layout.project_dir.join(&snap.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| IpcError::internal(e.to_string()))?;
    }
    std::fs::write(&target, &bytes).map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_snapshots_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_snapshots",
            serde_json::json!({ "path": "src/main.rs" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn get_snapshot_pair_diff_accepts_optional_ids() {
        let (svc, _dir) = crate::test_support::services();
        // Both ids missing → None/None → changed: false.
        let out = crate::dispatch("get_snapshot_pair_diff", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(out["changed"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn diff_endpoints_snapshot_vs_snapshot_classifies_changes() {
        let (svc, _dir) = crate::test_support::services();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let store = &svc.snapshot_store;
        let mk = |path: &str, hash: Option<&str>, snap: i64| oxplow_db::FileSnapshot {
            id: 0,
            stream_id: stream,
            path: path.into(),
            blob_hash: hash.map(|h| h.into()),
            size_bytes: 1,
            captured_at: oxplow_domain::Timestamp::now(),
            storage: oxplow_db::SnapshotStorage::Oxplow,
            snapshot_id: Some(snap),
            mtime_ms: None,
        };
        // p1: a + b baselined.
        let p1 = store.create_snapshot(stream).await.unwrap();
        store.capture(mk("a.txt", Some("h-a-1"), p1)).await.unwrap();
        store.capture(mk("b.txt", Some("h-b-1"), p1)).await.unwrap();
        // p2: a modified, c added, b deleted.
        let p2 = store.create_snapshot(stream).await.unwrap();
        store.capture(mk("a.txt", Some("h-a-2"), p2)).await.unwrap();
        store.capture(mk("c.txt", Some("h-c-1"), p2)).await.unwrap();
        store.capture(mk("b.txt", None, p2)).await.unwrap();

        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Snapshot { snapshot_id: p2 },
        )
        .await
        .unwrap();
        let by: std::collections::HashMap<_, _> = entries
            .iter()
            .map(|e| (e.path.as_str(), e.status.as_str()))
            .collect();
        assert_eq!(by.get("a.txt"), Some(&"modified"));
        assert_eq!(by.get("c.txt"), Some(&"added"));
        assert_eq!(by.get("b.txt"), Some(&"deleted"));
        // unchanged paths are omitted.
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn diff_endpoints_none_start_is_all_added() {
        let (svc, _dir) = crate::test_support::services();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let store = &svc.snapshot_store;
        let p1 = store.create_snapshot(stream).await.unwrap();
        store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "only.txt".into(),
                blob_hash: Some("h".into()),
                size_bytes: 1,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
            })
            .await
            .unwrap();
        let entries = super::diff_endpoints(
            &svc,
            None,
            super::DiffEndpoint::Snapshot { snapshot_id: p1 },
        )
        .await
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "added");
    }

    #[tokio::test]
    async fn diff_endpoints_commit_vs_commit_classifies_changes() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        std::fs::write(p.join("keep.txt"), "k").unwrap();
        std::fs::write(p.join("mod.txt"), "v1").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();
        std::fs::write(p.join("mod.txt"), "v2").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c2"]);
        let c2 = rev();

        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Commit { sha: c1 }),
            super::DiffEndpoint::Commit { sha: c2 },
        )
        .await
        .unwrap();
        assert!(entries
            .iter()
            .any(|e| e.path == "mod.txt" && e.status == "modified"));
        assert!(!entries.iter().any(|e| e.path == "keep.txt"));
    }

    #[tokio::test]
    async fn diff_endpoints_mixed_snapshot_vs_commit_normalizes_oxplow_blob() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };

        // Commit a file, then capture an oxplow-storage snapshot of the
        // SAME bytes (blob in the oxplow store, keyed by xxh3). The two
        // identities differ raw (xxh3 vs git oid) but must compare equal
        // after the snapshot side is normalized into git-oid space.
        let content = "line one\nline two\n";
        std::fs::write(p.join("a.txt"), content).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();

        let hash = svc.blobs.write(content.as_bytes()).unwrap();
        let p1 = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        svc.snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "a.txt".into(),
                blob_hash: Some(hash),
                size_bytes: content.len() as i64,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
            })
            .await
            .unwrap();

        let same = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Commit { sha: c1 },
        )
        .await
        .unwrap();
        assert!(
            !same.iter().any(|e| e.path == "a.txt"),
            "identical content across stores must not be a change: {same:?}"
        );

        // Now change the committed bytes → a.txt is modified.
        std::fs::write(p.join("a.txt"), "line one\nCHANGED\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c2"]);
        let c2 = rev();
        let changed = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Snapshot { snapshot_id: p1 }),
            super::DiffEndpoint::Commit { sha: c2 },
        )
        .await
        .unwrap();
        assert!(
            changed
                .iter()
                .any(|e| e.path == "a.txt" && e.status == "modified"),
            "differing content must be modified: {changed:?}"
        );
    }

    #[tokio::test]
    async fn diff_endpoints_working_tree_endpoint_detects_new_file() {
        let (svc, dir) = crate::test_support::services();
        std::fs::write(dir.path().join("w.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let entries = super::diff_endpoints(&svc, None, super::DiffEndpoint::Working)
            .await
            .unwrap();
        let w = entries
            .iter()
            .find(|e| e.path == "w.txt")
            .expect("new working-tree file should appear as added");
        assert_eq!(w.status, "added");
        assert_eq!(w.additions, 3, "three added lines: {w:?}");
        assert_eq!(w.deletions, 0);
    }

    #[tokio::test]
    async fn diff_endpoints_populates_line_counts_for_commits() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .status()
                .unwrap()
                .success());
        };
        let rev = || {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(p)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        std::fs::write(p.join("m.txt"), "a\nb\nc\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let c1 = rev();
        // line 2 b→B (1 del + 1 add), line 4 d appended (1 add).
        std::fs::write(p.join("m.txt"), "a\nB\nc\nd\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c2"]);
        let c2 = rev();

        let entries = super::diff_endpoints(
            &svc,
            Some(super::DiffEndpoint::Commit { sha: c1 }),
            super::DiffEndpoint::Commit { sha: c2 },
        )
        .await
        .unwrap();
        let m = entries
            .iter()
            .find(|e| e.path == "m.txt")
            .expect("m.txt changed");
        assert_eq!(m.status, "modified");
        assert_eq!((m.additions, m.deletions), (2, 1), "line counts: {m:?}");
    }

    #[tokio::test]
    async fn diff_endpoints_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "diff_endpoints",
            serde_json::json!({ "start": null, "end": { "kind": "commit", "sha": "HEAD" } }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn read_endpoint_files_content_reads_commit_blobs() {
        let (svc, dir) = crate::test_support::services();
        let p = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&p)
                .status()
                .unwrap()
                .success());
        };
        std::fs::write(p.join("a.txt"), "hello\nworld\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c1"]);
        let sha = {
            let o = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&p)
                .output()
                .unwrap();
            String::from_utf8(o.stdout).unwrap().trim().to_string()
        };

        let out = super::read_endpoint_files_content(
            &svc,
            super::DiffEndpoint::Commit { sha },
            vec!["a.txt".into(), "missing.txt".into()],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_deref(), Some("hello\nworld\n"));
        assert_eq!(out[1], None, "path absent at the endpoint reads as None");
    }

    #[tokio::test]
    async fn read_endpoint_files_content_reads_working_tree() {
        let (svc, dir) = crate::test_support::services();
        std::fs::write(dir.path().join("w.txt"), "live").unwrap();
        let out = super::read_endpoint_files_content(
            &svc,
            super::DiffEndpoint::Working,
            vec!["w.txt".into()],
        )
        .await
        .unwrap();
        assert_eq!(out[0].as_deref(), Some("live"));
    }
}
