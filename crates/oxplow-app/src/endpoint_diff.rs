//! Diffing two endpoints (a local-history snapshot, a git commit, or the
//! working tree) and reading file contents at an endpoint. Shared by the
//! `diff_endpoints` IPC and the change-analysis producer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use specta::Type;

use crate::blob_store::BlobStore;
use oxplow_db::{SnapshotStorage, SnapshotTree};
use oxplow_fs_watch::WorkspaceFilter;

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

/// The identity space a tree's `Cell`s compare in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    /// Snapshot content identity (`TreeEntry::identity`: the xxh3 of the
    /// bytes whatever the storage class; see `oxplow_db::snapshot_tree`).
    Snapshot,
    /// git blob oids (commit trees, and the working tree we build in this
    /// space).
    Git,
}

/// Where a path's raw bytes are, for reading and for normalizing into
/// git-oid space. Known from the row's storage class, never guessed.
#[derive(Clone)]
enum ContentSource {
    /// An xxh3 address in the oxplow blob store.
    Oxplow(String),
    /// A git blob oid in the git odb.
    Git(String),
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

/// `start_snap`/`end_snap` are the snapshot trees for snapshot endpoints;
/// when both sides are snapshots, run
/// `SqliteSnapshotStore::resolve_for_compare` on them first so a git-backed
/// and a blob-store copy of the same bytes compare equal.
pub fn compute_diff(
    start: Option<DiffEndpoint>,
    end: DiffEndpoint,
    start_snap: Option<SnapshotTree>,
    end_snap: Option<SnapshotTree>,
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
    snap_tree: Option<SnapshotTree>,
    project_dir: &Path,
    filter: &WorkspaceFilter,
) -> Result<(BTreeMap<String, Cell>, Space), String> {
    match ep {
        // EVERY arm applies the filter, not just `Working` (tsk177). Filtering
        // one side only makes a hidden path look DELETED — it's present in the
        // commit/snapshot tree and absent from the filtered worktree scan — and
        // it lands with the file's full line count as `deletions`. A filtered
        // path is out of scope, so it must be absent from both sides.
        DiffEndpoint::Snapshot { .. } => {
            let cells = snap_tree
                .unwrap_or_default()
                .into_iter()
                .filter(|(path, _)| !filter.ignore(Path::new(path), false))
                .map(|(path, entry)| {
                    let source = match (entry.storage, &entry.address) {
                        (SnapshotStorage::Oxplow, Some(a)) => ContentSource::Oxplow(a.clone()),
                        (SnapshotStorage::Git, Some(oid)) => ContentSource::Git(oid.clone()),
                        _ => ContentSource::Unreadable,
                    };
                    (
                        path,
                        Cell {
                            id: entry.identity(),
                            source,
                        },
                    )
                })
                .collect();
            Ok((cells, Space::Snapshot))
        }
        DiffEndpoint::Commit { sha } => {
            let tree = oxplow_git::tree_at_commit(project_dir, sha).map_err(|e| e.to_string())?;
            let cells = tree
                .into_iter()
                .filter(|(path, _)| !filter.ignore(Path::new(path), false))
                .map(|(path, oid)| {
                    (
                        path,
                        Cell {
                            id: oid.clone(),
                            source: ContentSource::Git(oid),
                        },
                    )
                })
                .collect();
            Ok((cells, Space::Git))
        }
        DiffEndpoint::Working => Ok((working_cells(project_dir, filter), Space::Git)),
    }
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
/// into git-oid space when crossing spaces: a git-backed snapshot cell is
/// already its oid, a blob-store cell needs a byte read + git rehash, an
/// oversize sentinel stays opaque. A failed read falls back to the raw id
/// (best-effort: it simply won't match, so the file reads as changed).
fn compare_id(cell: &Cell, normalize_to_git: bool, blobs: &BlobStore) -> String {
    if !normalize_to_git {
        return cell.id.clone();
    }
    match &cell.source {
        ContentSource::Oxplow(addr) => blobs
            .read(addr)
            .ok()
            .as_deref()
            .and_then(oxplow_git::git_blob_oid)
            .unwrap_or_else(|| cell.id.clone()),
        ContentSource::Git(oid) => oid.clone(),
        _ => cell.id.clone(),
    }
}

/// Raw bytes for a cell, for line counting. `None` for oversize / pruned
/// content or a failed read.
fn read_cell(cell: &Cell, blobs: &BlobStore, project_dir: &Path) -> Option<Vec<u8>> {
    match &cell.source {
        ContentSource::Oxplow(addr) => blobs.read(addr).ok(),
        ContentSource::Git(oid) => oxplow_git::read_blob(project_dir, oid),
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

/// The UTF-8 (lossy) contents of `paths` at `endpoint`; `None` for a path
/// that doesn't exist there or can't be read. `snap_tree` is the snapshot's
/// tree when `endpoint` is a snapshot.
pub fn endpoint_contents(
    endpoint: &DiffEndpoint,
    snap_tree: Option<SnapshotTree>,
    project_dir: &Path,
    blobs: &BlobStore,
    filter: &WorkspaceFilter,
    paths: Vec<String>,
) -> Result<Vec<Option<String>>, String> {
    let (cells, _space) = cells_for_endpoint(endpoint, snap_tree, project_dir, filter)?;
    Ok(paths
        .into_iter()
        .map(|p| {
            cells
                .get(&p)
                .and_then(|cell| read_cell(cell, blobs, project_dir))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(
        f: &crate::test_fixtures::EffortFixture,
        ep: DiffEndpoint,
        paths: &[&str],
    ) -> Vec<Option<String>> {
        let root = f.svc.layout.project_dir.clone();
        let filter =
            WorkspaceFilter::for_project(&root, Vec::<String>::new(), Vec::<String>::new());
        endpoint_contents(
            &ep,
            None,
            &root,
            &f.svc.blobs,
            &filter,
            paths.iter().map(|p| p.to_string()).collect(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_commit_endpoint_reads_blobs_and_misses_absent_paths() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "hello\nworld\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "c1");
        let out = read(&f, DiffEndpoint::Commit { sha }, &["a.txt", "missing.txt"]);
        assert_eq!(out, vec![Some("hello\nworld\n".to_string()), None]);
    }

    #[tokio::test]
    async fn the_working_endpoint_reads_the_disk() {
        let f = crate::test_fixtures::services_with_effort().await;
        std::fs::write(f.svc.layout.project_dir.join("w.txt"), "live").unwrap();
        assert_eq!(
            read(&f, DiffEndpoint::Working, &["w.txt"]),
            vec![Some("live".to_string())]
        );
    }
}
