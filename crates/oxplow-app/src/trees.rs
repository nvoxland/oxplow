//! Every version of a workspace's tree behind one interface
//! (`.context/vcs.md` "Trees"): list, read and diff the working tree, a
//! local-history snapshot, or a VCS revision, named by
//! [`Revision`]. Diff views, change analysis and the code-quality scans
//! read through here, so none of them knows where the bytes live.
//!
//! Every tree honours the workspace filter (`generated:` config and
//! `.gitignore`), on every side of a diff: a path filtered on one side
//! only would show as deleted.
//!
//! **Identity spaces.** A snapshot compares by content hash (xxh3); VCS
//! trees and the working tree compare by the VCS's object ids. A diff of
//! two sides in the same space compares ids without reading a byte; a
//! mixed diff normalizes the snapshot side into VCS ids
//! (`ObjectStore::id_of` over its bytes).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use oxplow_config::OxplowConfig;
use oxplow_db::{SnapshotStorage, SnapshotTree, SqliteSnapshotStore};
use oxplow_domain::vcs::{FileStatus, ObjectId, Revision, Vcs};
use oxplow_domain::{ChangeStatus, DomainError};
use oxplow_fs_watch::WorkspaceFilter;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use specta::Type;

use crate::blob_store::BlobStore;

/// One path that differs between two revisions. `additions`/`deletions`
/// are line counts, `0` for binary or unreadable content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    /// `added`, `modified` or `deleted`.
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
}

/// Where a path's bytes are.
#[derive(Clone, Debug)]
enum Source {
    /// An address in the oxplow blob store.
    Blob(String),
    /// An object in the VCS's store.
    Object(ObjectId),
    /// A file on disk.
    File(PathBuf),
    /// Oversize or pruned: no bytes.
    Unreadable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Snapshot,
    Vcs,
}

/// A path's content within one tree: its identity in the tree's space,
/// and where to read it.
#[derive(Clone, Debug)]
struct Cell {
    id: String,
    source: Source,
}

type Cells = BTreeMap<String, Cell>;

pub struct Trees {
    vcs: Arc<dyn Vcs>,
    snapshots: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    config: Arc<RwLock<OxplowConfig>>,
}

fn invalid(m: impl Into<String>) -> DomainError {
    DomainError::Invalid(m.into())
}

async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, DomainError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| DomainError::Invariant(format!("tree read didn't finish: {e}")))
}

/// `path` as a workspace-relative path that can't leave the workspace.
fn relative(path: &str) -> Result<&str, DomainError> {
    let p = Path::new(path);
    if path.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(invalid(format!(
            "`{path}` isn't a path inside the workspace"
        )));
    }
    Ok(path)
}

impl Trees {
    pub fn new(
        vcs: Arc<dyn Vcs>,
        snapshots: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
    ) -> Self {
        Self {
            vcs,
            snapshots,
            blobs,
            config,
        }
    }

    fn filter(&self, ws: &Path) -> WorkspaceFilter {
        let cfg = self.config.read().unwrap_or_else(|e| e.into_inner());
        WorkspaceFilter::for_project(ws, &cfg.generated.exclude, &cfg.generated.include)
    }

    /// The VCS revision `rev` names, refusing another VCS's.
    fn vcs_rev<'a>(&self, kind: &str, rev: &'a str) -> Result<&'a str, DomainError> {
        if kind != self.vcs.rev_kind() {
            return Err(invalid(format!(
                "`{kind}:{rev}` is a {kind} revision; this workspace is under {}",
                self.vcs.rev_kind()
            )));
        }
        Ok(rev)
    }

    /// The VCS revision `snapshot`'s tree equals — set when it was taken
    /// on a clean workspace at its head, or when the head later moved onto
    /// an unchanged tree.
    pub async fn revision_of(&self, snapshot: i64) -> Result<Option<Revision>, DomainError> {
        self.snapshots.get_snapshot_revision(snapshot).await
    }

    /// The snapshots whose trees equal VCS revision `rev` (resolved in
    /// `ws` first, so `git:HEAD` and a short id work), oldest first.
    pub async fn snapshots_at(&self, ws: &Path, rev: &Revision) -> Result<Vec<i64>, DomainError> {
        let Revision::Vcs { kind, rev } = rev else {
            return Err(invalid(format!("`{rev}` isn't a VCS revision")));
        };
        let full = self.vcs.resolve(ws, self.vcs_rev(kind, rev)?).await?;
        self.snapshots
            .snapshots_at(&Revision::Vcs {
                kind: kind.clone(),
                rev: full,
            })
            .await
    }

    /// Every file in `rev`, sorted.
    pub async fn files_at(&self, ws: &Path, rev: &Revision) -> Result<Vec<String>, DomainError> {
        Ok(self.cells(ws, rev).await?.0.into_keys().collect())
    }

    /// The bytes of `path` at `rev`; `None` when it isn't there (or has
    /// no bytes: oversize, pruned).
    pub async fn read_at(
        &self,
        ws: &Path,
        rev: &Revision,
        path: &str,
    ) -> Result<Option<Vec<u8>>, DomainError> {
        let path = relative(path)?;
        let source = match rev {
            Revision::Working => Source::File(ws.join(path)),
            Revision::Snapshot(id) => match self.snapshots.content_ref_for_path(*id, path).await? {
                Some(r) => snapshot_source(r.storage, Some(&r.hash)),
                None => return Ok(None),
            },
            Revision::Vcs { kind, rev } => {
                let rev = self.vcs_rev(kind, rev)?;
                match self.vcs.files_at(ws, rev).await?.remove(path) {
                    Some(id) => Source::Object(id),
                    None => return Ok(None),
                }
            }
        };
        self.read(ws, &source).await
    }

    /// The text of every file in `rev` that `keep` accepts; binary and
    /// unreadable files are left out. What the code-quality scans parse.
    pub async fn corpus(
        &self,
        ws: &Path,
        rev: &Revision,
        keep: impl Fn(&str) -> bool,
    ) -> Result<Vec<(String, String)>, DomainError> {
        let (cells, _) = self.cells(ws, rev).await?;
        let mut out = Vec::new();
        for (path, cell) in cells.into_iter().filter(|(p, _)| keep(p)) {
            if let Some(bytes) = self.read(ws, &cell.source).await? {
                if let Ok(text) = String::from_utf8(bytes) {
                    out.push((path, text));
                }
            }
        }
        Ok(out)
    }

    /// What changed from `from` to `to` (from nothing when `from` is
    /// `None`), sorted by path, with line counts.
    pub async fn diff(
        &self,
        ws: &Path,
        from: Option<&Revision>,
        to: &Revision,
    ) -> Result<Vec<DiffEntry>, DomainError> {
        let ids = |c: &Cells| -> BTreeMap<String, String> {
            c.iter().map(|(p, c)| (p.clone(), c.id.clone())).collect()
        };
        let (before, after, changes) = match (from, to) {
            // Two snapshots: settle un-hashed VCS-backed rows first, so
            // equal bytes compare equal.
            (Some(Revision::Snapshot(a)), Revision::Snapshot(b)) => {
                let mut ta = self.snapshots.tree_at(*a).await?;
                let mut tb = self.snapshots.tree_at(*b).await?;
                self.snapshots.resolve_for_compare(&mut ta, &mut tb).await?;
                let filter = self.filter(ws);
                let before = snapshot_cells(ta, &filter);
                let after = snapshot_cells(tb, &filter);
                let changes = oxplow_domain::diff_trees(&ids(&before), &ids(&after));
                (before, after, changes)
            }
            _ => {
                let (mut before, before_space) = match from {
                    Some(rev) => {
                        let (c, s) = self.cells(ws, rev).await?;
                        (c, Some(s))
                    }
                    None => (Cells::new(), None),
                };
                let (mut after, after_space) = self.cells(ws, to).await?;
                if before_space.is_some_and(|s| s != after_space) {
                    self.normalize(ws, &mut before).await?;
                    self.normalize(ws, &mut after).await?;
                }
                let changes = oxplow_domain::diff_trees(&ids(&before), &ids(&after));
                (before, after, changes)
            }
        };
        let mut out = Vec::with_capacity(changes.len());
        for c in changes {
            let old = match before.get(&c.path) {
                Some(cell) => self.read(ws, &cell.source).await?,
                None => None,
            };
            let new = match after.get(&c.path) {
                Some(cell) => self.read(ws, &cell.source).await?,
                None => None,
            };
            let (additions, deletions) = count_lines(old.as_deref(), new.as_deref());
            out.push(DiffEntry {
                path: c.path,
                status: match c.status {
                    ChangeStatus::Added => FileStatus::Added,
                    ChangeStatus::Modified => FileStatus::Modified,
                    ChangeStatus::Deleted => FileStatus::Deleted,
                },
                additions,
                deletions,
            });
        }
        Ok(out)
    }

    /// `rev`'s filtered `path → Cell` tree and its identity space.
    async fn cells(&self, ws: &Path, rev: &Revision) -> Result<(Cells, Space), DomainError> {
        let filter = self.filter(ws);
        let keep = |p: &str| !filter.ignore(Path::new(p), false);
        match rev {
            Revision::Snapshot(id) => {
                let tree = self.snapshots.tree_at(*id).await?;
                Ok((snapshot_cells(tree, &filter), Space::Snapshot))
            }
            Revision::Vcs { kind, rev } => {
                let rev = self.vcs_rev(kind, rev)?;
                let cells = self
                    .vcs
                    .files_at(ws, rev)
                    .await?
                    .into_iter()
                    .filter(|(p, _)| keep(p))
                    .map(|(p, id)| {
                        (
                            p,
                            Cell {
                                id: id.0.clone(),
                                source: Source::Object(id),
                            },
                        )
                    })
                    .collect();
                Ok((cells, Space::Vcs))
            }
            Revision::Working => Ok((self.working_cells(ws, filter).await?, Space::Vcs)),
        }
    }

    /// The working tree in VCS-id space. A file the VCS reports clean
    /// reuses its head object id (no read); anything else is hashed.
    async fn working_cells(
        &self,
        ws: &Path,
        filter: WorkspaceFilter,
    ) -> Result<Cells, DomainError> {
        let head = match self.vcs.head(ws).await.ok().and_then(|h| h.revision) {
            Some(rev) => self.vcs.files_at(ws, &rev).await?,
            None => BTreeMap::new(),
        };
        let changed: BTreeSet<String> = match self.vcs.status(ws).await {
            Ok(s) => s.entries.into_iter().map(|e| e.path).collect(),
            Err(_) => BTreeSet::new(),
        };
        let clean: BTreeMap<String, ObjectId> = head
            .into_iter()
            .filter(|(p, _)| !changed.contains(p))
            .collect();
        let (root, objects) = (ws.to_path_buf(), self.vcs.object_store(ws));
        blocking(move || {
            let mut out = Cells::new();
            for entry in walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || {
                        let rel = e.path().strip_prefix(&root).unwrap_or(e.path());
                        !filter.ignore(rel, e.file_type().is_dir())
                    }
                })
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let Ok(rel) = entry.path().strip_prefix(&root) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let id = match clean.get(&rel) {
                    Some(id) => id.0.clone(),
                    None => match std::fs::read(entry.path()) {
                        Ok(bytes) => objects.id_of(&bytes).0,
                        Err(_) => continue,
                    },
                };
                out.insert(
                    rel,
                    Cell {
                        id,
                        source: Source::File(entry.path().to_path_buf()),
                    },
                );
            }
            out
        })
        .await
    }

    /// Re-key snapshot-space cells into VCS ids (VCS-space cells are
    /// already there).
    async fn normalize(&self, ws: &Path, cells: &mut Cells) -> Result<(), DomainError> {
        for cell in cells.values_mut() {
            match &cell.source {
                Source::Blob(_) => {
                    if let Some(bytes) = self.read(ws, &cell.source).await? {
                        cell.id = self.vcs.object_store(ws).id_of(&bytes).0;
                    }
                }
                Source::Object(id) => cell.id = id.0.clone(),
                Source::File(_) | Source::Unreadable => {}
            }
        }
        Ok(())
    }

    async fn read(&self, ws: &Path, source: &Source) -> Result<Option<Vec<u8>>, DomainError> {
        match source {
            Source::Blob(addr) => {
                let (blobs, addr) = (self.blobs.clone(), addr.clone());
                blocking(move || blobs.read(&addr).ok()).await
            }
            Source::Object(id) => {
                let (objects, id) = (self.vcs.object_store(ws), id.clone());
                blocking(move || objects.read(&id)).await
            }
            Source::File(path) => {
                let path = path.clone();
                blocking(move || std::fs::read(path).ok()).await
            }
            Source::Unreadable => Ok(None),
        }
    }
}

/// A snapshot's files, filtered, in snapshot space.
fn snapshot_cells(tree: SnapshotTree, filter: &WorkspaceFilter) -> Cells {
    tree.into_iter()
        .filter(|(p, _)| !filter.ignore(Path::new(p), false))
        .map(|(path, entry)| {
            let id = entry.identity();
            let source = snapshot_source(entry.storage, entry.address.as_deref());
            (path, Cell { id, source })
        })
        .collect()
}

fn snapshot_source(storage: SnapshotStorage, address: Option<&str>) -> Source {
    match (storage, address) {
        (SnapshotStorage::Oxplow, Some(a)) => Source::Blob(a.to_string()),
        (SnapshotStorage::Git, Some(oid)) => Source::Object(ObjectId(oid.to_string())),
        _ => Source::Unreadable,
    }
}

fn count_lines(old: Option<&[u8]>, new: Option<&[u8]>) -> (u32, u32) {
    let binary = |b: &&[u8]| b.contains(&0);
    if old.filter(binary).is_some() || new.filter(binary).is_some() {
        return (0, 0);
    }
    let text = |b: Option<&[u8]>| {
        b.map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    };
    let (old, new) = (text(old), text(new));
    let mut counts = (0u32, 0u32);
    for change in TextDiff::from_lines(old.as_str(), new.as_str()).iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => counts.0 += 1,
            ChangeTag::Delete => counts.1 += 1,
            ChangeTag::Equal => {}
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{commit_all, services_with_effort, EffortFixture};
    use oxplow_db::FileSnapshot;
    use oxplow_domain::stores::StreamStore as _;

    fn by_path(entries: &[DiffEntry]) -> BTreeMap<&str, (FileStatus, u32, u32)> {
        entries
            .iter()
            .map(|e| (e.path.as_str(), (e.status, e.additions, e.deletions)))
            .collect()
    }

    /// A snapshot of `files` (path → content; `None` = a deletion), with
    /// the bytes in the blob store.
    async fn snapshot(f: &EffortFixture, files: &[(&str, Option<&str>)]) -> i64 {
        let svc = &f.svc;
        let stream = svc.stream_store.list().await.unwrap()[0].id;
        let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        for (path, content) in files {
            let hash = content.map(|c| svc.blobs.write(c.as_bytes()).unwrap());
            svc.snapshot_store
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream,
                    path: (*path).into(),
                    blob_hash: hash,
                    size_bytes: content.map_or(0, |c| c.len() as i64),
                    captured_at: oxplow_domain::Timestamp::now(),
                    storage: if content.is_some() {
                        SnapshotStorage::Oxplow
                    } else {
                        SnapshotStorage::Deleted
                    },
                    snapshot_id: Some(id),
                    mtime_ms: None,
                    content_hash: None,
                })
                .await
                .unwrap();
        }
        id
    }

    /// P5.B2 (tsk521): a snapshot reads through `Trees` — its files, one
    /// file's bytes, and its text corpus — whatever the disk holds now.
    #[tokio::test]
    async fn a_snapshot_reads_through_trees() {
        let f = services_with_effort().await;
        let (trees, ws) = (&f.svc.trees, f.svc.layout.project_dir.clone());
        let snap = snapshot(&f, &[("a.rs", Some("fn a() {}\n")), ("b.txt", Some("b\n"))]).await;
        std::fs::write(ws.join("a.rs"), "changed\n").unwrap();
        let rev = Revision::Snapshot(snap);
        assert_eq!(
            trees.files_at(&ws, &rev).await.unwrap(),
            vec!["a.rs", "b.txt"]
        );
        assert_eq!(
            trees.read_at(&ws, &rev, "a.rs").await.unwrap().as_deref(),
            Some(&b"fn a() {}\n"[..])
        );
        assert_eq!(trees.read_at(&ws, &rev, "nope.rs").await.unwrap(), None);
        let corpus = trees
            .corpus(&ws, &rev, |p| p.ends_with(".rs"))
            .await
            .unwrap();
        assert_eq!(
            corpus,
            vec![("a.rs".to_string(), "fn a() {}\n".to_string())]
        );
        assert!(trees.read_at(&ws, &rev, "../x").await.is_err());
    }

    /// Conformance 7 (P5.B3, tsk522): a snapshot taken on a clean
    /// workspace maps to its head revision and back, and diffs empty
    /// against it; one taken with an edit maps to none.
    #[tokio::test]
    async fn a_clean_snapshot_maps_to_its_revision_and_diffs_empty() {
        use crate::snapshot_capture::{SnapshotCaptureService, TakeRequest};
        use oxplow_domain::snapshot::SnapshotTrigger;
        let f = services_with_effort().await;
        let (trees, ws) = (&f.svc.trees, f.svc.layout.project_dir.clone());
        std::fs::write(ws.join("a.txt"), "one\n").unwrap();
        let head = commit_all(&ws, "c1");
        let capture = SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            ws.clone(),
            f.svc.vcs.clone(),
            f.svc.stream_store.list().await.unwrap()[0].id,
            1_000_000,
            // The filter production captures with — what `Trees` reads by.
            WorkspaceFilter::for_project(&ws, Vec::<String>::new(), Vec::<String>::new()),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        let take = |capture: SnapshotCaptureService| async move {
            capture.enqueue_startup_diff().await.unwrap();
            capture
                .request_snapshot(TakeRequest {
                    trigger: SnapshotTrigger::Manual,
                    thread_id: None,
                    turn_id: None,
                    effort_id: None,
                    budget: None,
                })
                .await
                .unwrap()
                .expect("a snapshot")
        };
        let status = f.svc.vcs.status(&ws).await.unwrap();
        assert!(
            status.entries.is_empty(),
            "the fixture commits clean: {status:?}"
        );
        let clean = take(capture.clone()).await;
        let rev = Revision::git(head);
        assert_eq!(trees.revision_of(clean).await.unwrap(), Some(rev.clone()));
        assert_eq!(trees.snapshots_at(&ws, &rev).await.unwrap(), vec![clean]);
        assert_eq!(
            trees
                .snapshots_at(&ws, &Revision::git("HEAD"))
                .await
                .unwrap(),
            vec![clean]
        );
        let d = trees
            .diff(&ws, Some(&Revision::Snapshot(clean)), &rev)
            .await
            .unwrap();
        assert!(d.is_empty(), "{d:?}");

        std::fs::write(ws.join("a.txt"), "two\n").unwrap();
        let dirty = take(capture).await;
        assert_ne!(dirty, clean);
        assert_eq!(trees.revision_of(dirty).await.unwrap(), None);
    }

    /// Another VCS's revision is refused, naming this workspace's.
    #[tokio::test]
    async fn another_vcs_revision_is_refused() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        let jj = Revision::Vcs {
            kind: "jj".into(),
            rev: "@".into(),
        };
        let err = f
            .svc
            .trees
            .files_at(&ws, &jj)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("under git"), "{err}");
    }

    #[tokio::test]
    async fn two_snapshots_diff_to_what_changed() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        let p1 = snapshot(&f, &[("a.txt", Some("a1\n")), ("b.txt", Some("b\n"))]).await;
        let p2 = snapshot(
            &f,
            &[
                ("a.txt", Some("a2\n")),
                ("c.txt", Some("c\n")),
                ("b.txt", None),
            ],
        )
        .await;
        let d = f
            .svc
            .trees
            .diff(&ws, Some(&Revision::Snapshot(p1)), &Revision::Snapshot(p2))
            .await
            .unwrap();
        assert_eq!(
            by_path(&d),
            BTreeMap::from([
                ("a.txt", (FileStatus::Modified, 1, 1)),
                ("b.txt", (FileStatus::Deleted, 0, 1)),
                ("c.txt", (FileStatus::Added, 1, 0)),
            ])
        );
        // From nothing, everything is added.
        let all = f
            .svc
            .trees
            .diff(&ws, None, &Revision::Snapshot(p1))
            .await
            .unwrap();
        assert!(all.iter().all(|e| e.status == FileStatus::Added), "{all:?}");
    }

    #[tokio::test]
    async fn two_commits_diff_with_line_counts() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        std::fs::write(ws.join("keep.txt"), "k\n").unwrap();
        std::fs::write(ws.join("m.txt"), "a\nb\nc\n").unwrap();
        let c1 = commit_all(&ws, "c1");
        std::fs::write(ws.join("m.txt"), "a\nB\nc\nd\n").unwrap();
        let c2 = commit_all(&ws, "c2");
        let d = f
            .svc
            .trees
            .diff(&ws, Some(&Revision::git(c1)), &Revision::git(c2))
            .await
            .unwrap();
        assert_eq!(
            by_path(&d),
            BTreeMap::from([("m.txt", (FileStatus::Modified, 2, 1))])
        );
    }

    /// A snapshot and a commit holding the same bytes compare equal once
    /// the snapshot side is normalized into VCS ids.
    #[tokio::test]
    async fn a_snapshot_and_a_commit_compare_by_content() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        std::fs::write(ws.join("a.txt"), "one\ntwo\n").unwrap();
        let c1 = commit_all(&ws, "c1");
        let snap = Revision::Snapshot(snapshot(&f, &[("a.txt", Some("one\ntwo\n"))]).await);
        let same = f
            .svc
            .trees
            .diff(&ws, Some(&snap), &Revision::git(c1))
            .await
            .unwrap();
        assert!(!same.iter().any(|e| e.path == "a.txt"), "{same:?}");
        std::fs::write(ws.join("a.txt"), "one\nCHANGED\n").unwrap();
        let c2 = commit_all(&ws, "c2");
        let changed = f
            .svc
            .trees
            .diff(&ws, Some(&snap), &Revision::git(c2))
            .await
            .unwrap();
        assert_eq!(
            by_path(&changed).get("a.txt"),
            Some(&(FileStatus::Modified, 1, 1))
        );
    }

    #[tokio::test]
    async fn the_working_tree_diffs_against_its_head() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        std::fs::write(ws.join("tracked.txt"), "t\n").unwrap();
        let head = commit_all(&ws, "c1");
        std::fs::write(ws.join("w.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let d = f
            .svc
            .trees
            .diff(&ws, Some(&Revision::git(head)), &Revision::Working)
            .await
            .unwrap();
        let d = by_path(&d);
        assert_eq!(d.get("w.txt"), Some(&(FileStatus::Added, 3, 0)));
        assert!(!d.contains_key("tracked.txt"), "{d:?}");
    }

    /// tsk177: a filtered path is out of scope on both sides — never
    /// "deleted" because the working tree hides it.
    #[tokio::test]
    async fn a_filtered_path_is_not_reported_deleted() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(ws.join(".oxplow")).unwrap();
        std::fs::write(
            ws.join(".oxplow/.gitignore"),
            "*\n!.gitignore\n!project.yaml\n",
        )
        .unwrap();
        std::fs::write(ws.join(".oxplow/project.yaml"), "collection:\n  a: 1\n").unwrap();
        std::fs::write(ws.join("normal.txt"), "x\n").unwrap();
        let c1 = commit_all(&ws, "c1");
        let d = f
            .svc
            .trees
            .diff(&ws, Some(&Revision::git(c1)), &Revision::Working)
            .await
            .unwrap();
        let deleted: Vec<_> = d
            .iter()
            .filter(|e| e.status == FileStatus::Deleted)
            .collect();
        assert!(deleted.is_empty(), "{deleted:?}");
    }

    /// tsk552: a diff between two commits honours the workspace filter
    /// like every other pair — an excluded path isn't reported.
    #[tokio::test]
    async fn a_commit_diff_leaves_out_filtered_paths() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        f.svc.config.write().unwrap().generated.exclude = vec!["dist".into()];
        std::fs::create_dir_all(ws.join("dist")).unwrap();
        std::fs::write(ws.join("dist/app.js"), "v1\n").unwrap();
        std::fs::write(ws.join("src.txt"), "a\n").unwrap();
        let c1 = commit_all(&ws, "c1");
        std::fs::write(ws.join("dist/app.js"), "v2\n").unwrap();
        std::fs::write(ws.join("src.txt"), "b\n").unwrap();
        let c2 = commit_all(&ws, "c2");
        let d = f
            .svc
            .trees
            .diff(&ws, Some(&Revision::git(c1)), &Revision::git(c2))
            .await
            .unwrap();
        assert_eq!(
            by_path(&d),
            BTreeMap::from([("src.txt", (FileStatus::Modified, 1, 1))])
        );
    }
}
