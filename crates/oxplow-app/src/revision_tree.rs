//! The tree a script run reads (`tree.read`) when its event is about a
//! particular code state: `Anchors::revision`, which `change.analyzed` sets
//! to its change's head. `working` or none reads what an anchor-less run
//! reads (the snapshot it is anchored to, else its stream's latest); `snap:<id>`
//! that snapshot's reconstructed tree; `git:<sha>` the files at that commit
//! through the VCS, as the analyzer reads head-side contents. Read lazily:
//! only when the script uses `files()` or `grep_tree()`.

use std::sync::{Arc, Mutex};

use oxplow_domain::events::Anchors;
use oxplow_domain::vcs::Revision;

use crate::services::{Tree, TreeSource};
use crate::trees::Trees;
use crate::worktrees::WorktreeRouter;

/// Text files larger than this are left out of a commit's tree, as the
/// worktree reader leaves them out.
const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;

/// A commit's tree, by the stream and revision it was read for.
type Read = ((Option<i64>, String), Tree);

pub struct RevisionTrees {
    /// What a run with no revision (or `working`) reads.
    inner: Arc<dyn TreeSource>,
    trees: Arc<Trees>,
    worktrees: Arc<WorktreeRouter>,
    /// The last commit tree read, by stream and revision: the runs one
    /// event triggers share it.
    last: Mutex<Option<Read>>,
}

impl RevisionTrees {
    pub fn new(
        inner: Arc<dyn TreeSource>,
        trees: Arc<Trees>,
        worktrees: Arc<WorktreeRouter>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            trees,
            worktrees,
            last: Mutex::new(None),
        })
    }
}

#[async_trait::async_trait]
impl TreeSource for RevisionTrees {
    async fn tree(&self, anchors: &Anchors) -> Result<Tree, String> {
        let Some(revision) = anchors.revision.as_deref() else {
            return self.inner.tree(anchors).await;
        };
        let rev: Revision = revision
            .parse()
            .map_err(|e| format!("`tree.read`: the event's revision: {e}"))?;
        match rev {
            Revision::Working => self.inner.tree(anchors).await,
            Revision::Snapshot(id) => {
                let mut at = anchors.clone();
                at.snapshot_id = Some(id);
                self.inner.tree(&at).await
            }
            rev @ Revision::Vcs { .. } => {
                let stream = anchors.stream_id.map(|s| s.value());
                let key = (stream, revision.to_string());
                if let Some((k, tree)) = self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
                {
                    if k == key {
                        return Ok(tree);
                    }
                }
                let root = match stream {
                    Some(stream) => {
                        crate::change_analysis::stream_root(&self.worktrees, stream).await
                    }
                    None => crate::collector_runner::primary_root(&self.worktrees).await,
                };
                let files = self
                    .trees
                    .corpus(&root, &rev, |_| true)
                    .await
                    .map_err(|e| format!("`tree.read`: {revision}: {e}"))?;
                let tree: Tree = Arc::new(
                    files
                        .into_iter()
                        .filter(|(_, text)| text.len() <= MAX_FILE_BYTES)
                        .collect(),
                );
                *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, tree.clone()));
                Ok(tree)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::StreamId;

    fn source(svc: &crate::Services) -> Arc<RevisionTrees> {
        RevisionTrees::new(
            Arc::new(svc.metrics.clone()),
            svc.trees.clone(),
            svc.worktrees.clone(),
        )
    }

    fn at(revision: Option<String>) -> Anchors {
        at_stream(StreamId::new(1), revision)
    }

    fn at_stream(stream: StreamId, revision: Option<String>) -> Anchors {
        Anchors {
            stream_id: Some(stream),
            revision,
            ..Anchors::default()
        }
    }

    /// A run for an event about a commit reads that commit's files, not the
    /// worktree's or the latest snapshot's: a file that differs between them
    /// reads as it was at the commit; the same event with no revision reads
    /// the latest snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_git_revision_reads_that_commits_tree() {
        let f = crate::test_fixtures::services_with_thread().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        let first = crate::test_fixtures::commit_all(&root, "first");
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        crate::test_fixtures::commit_all(&root, "second");
        std::fs::write(root.join("a.txt"), "three\n").unwrap();
        let trees = source(&f.svc);
        let tree = trees.tree(&at(Some(format!("git:{first}")))).await.unwrap();
        assert_eq!(tree.get("a.txt").map(String::as_str), Some("one\n"));
        let again = trees.tree(&at(Some(format!("git:{first}")))).await.unwrap();
        assert!(Arc::ptr_eq(&tree, &again), "the runs of one event share it");
        let head = trees.tree(&at(Some("git:HEAD".into()))).await.unwrap();
        assert_eq!(head.get("a.txt").map(String::as_str), Some("two\n"));
        let bad = trees.tree(&at(Some("nonsense".into()))).await.unwrap_err();
        assert!(bad.contains("the event's revision"), "{bad}");
    }

    /// `snap:<id>` reads that snapshot's tree, whatever the latest holds.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_snapshot_revision_reads_that_snapshots_tree() {
        use oxplow_domain::snapshot::SnapshotTrigger;
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let stream = StreamId::new(1);
        let capture = svc.snapshot_captures.primary().expect("a primary capture");
        capture.await_initial_ready().await;
        let mut ids = Vec::new();
        for body in ["one\n", "two\n"] {
            std::fs::write(root.join("a.txt"), body).unwrap();
            capture.enqueue_startup_diff().await.unwrap();
            ids.push(
                capture
                    .request_snapshot(SnapshotTrigger::Manual)
                    .await
                    .unwrap()
                    .expect("a snapshot of the edit"),
            );
        }
        assert_ne!(ids[0], ids[1]);
        let trees = source(svc);
        let old = trees
            .tree(&at(Some(format!("snap:{}", ids[0]))))
            .await
            .unwrap();
        assert_eq!(old.get("a.txt").map(String::as_str), Some("one\n"));
        let latest = trees.tree(&at_stream(stream, None)).await.unwrap();
        assert_eq!(latest.get("a.txt").map(String::as_str), Some("two\n"));
        let working = trees
            .tree(&at_stream(stream, Some("working".into())))
            .await
            .unwrap();
        assert_eq!(working.get("a.txt").map(String::as_str), Some("two\n"));
    }

    /// `change.analyzed` anchors its change's head revision.
    #[tokio::test(flavor = "multi_thread")]
    async fn change_analyzed_is_anchored_to_the_changes_head() {
        let f = crate::test_fixtures::services_with_thread().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        crate::test_fixtures::commit_all(&root, "first");
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "second");
        crate::change_analysis::ensure_change(
            &f.svc,
            crate::change_analysis::ChangeTarget::Commit {
                sha: sha.clone(),
                stream_id: None,
            },
        )
        .await
        .unwrap();
        let revisions: Vec<Option<String>> = f
            .svc
            .db
            .read(|c| {
                let mut st = c
                    .prepare("SELECT revision FROM event_log WHERE type = 'change.analyzed' ORDER BY seq")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<_, _>>().map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert!(!revisions.is_empty());
        assert!(
            revisions
                .iter()
                .all(|r| r.as_deref() == Some(format!("git:{sha}").as_str())),
            "{revisions:?} vs {sha}"
        );
    }
}
