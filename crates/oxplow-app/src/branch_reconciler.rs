//! Keeps each stream's recorded branch equal to the branch its workspace
//! has checked out (`.context/vcs.md`). A checkout from a terminal, or
//! one made while oxplow was off, never touches the stream row on its
//! own; without this the branch chip and every other reader of
//! `stream.branch` keep showing the branch the stream was created on.
//!
//! It runs once per stream at boot and again on each `VcsRefsChanged`.
//! A detached head leaves the row as it is.

use std::path::Path;
use std::sync::Arc;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::vcs::Vcs;
use oxplow_domain::StreamId;
use tracing::{debug, warn};

use crate::ref_moves::Moved;
use crate::worktrees::WorktreeRouter;

pub struct BranchReconciler {
    router: Arc<WorktreeRouter>,
    vcs: Arc<dyn Vcs>,
    streams: Arc<dyn StreamStore>,
    /// What it listens to: the VCS watcher's ref moves.
    ref_moves: crate::ref_moves::RefMoves,
}

impl BranchReconciler {
    pub fn new(
        router: Arc<WorktreeRouter>,
        vcs: Arc<dyn Vcs>,
        streams: Arc<dyn StreamStore>,
        ref_moves: crate::ref_moves::RefMoves,
    ) -> Self {
        Self {
            router,
            vcs,
            streams,
            ref_moves,
        }
    }

    /// Reconcile every stream now, then each one whose refs move, for the
    /// life of the process.
    pub fn spawn(self: Arc<Self>) {
        let mut rx = self.ref_moves.subscribe();
        tokio::spawn(async move {
            self.reconcile_all().await;
            while let Some(moved) = rx.recv().await {
                match moved {
                    Moved::Stream(stream_id) => {
                        let path = self.router.resolve(Some(&stream_id.to_string())).await;
                        self.reconcile(&stream_id, &path).await;
                    }
                    // Any stream may have moved.
                    Moved::Missed => self.reconcile_all().await,
                }
            }
        });
    }

    async fn reconcile_all(&self) {
        match self.router.all().await {
            Ok(all) => {
                for (id, path) in all {
                    self.reconcile(&id, &path).await;
                }
            }
            Err(error) => warn!(%error, "couldn't list the streams to reconcile"),
        }
    }

    /// Persist the branch `worktree` has checked out onto `stream_id`'s
    /// row when they differ.
    pub async fn reconcile(&self, stream_id: &StreamId, worktree: &Path) {
        let Ok(head) = self.vcs.head(worktree).await else {
            return;
        };
        let Some(detected) = head.branch else {
            return;
        };
        let Ok(Some(stored)) = self.streams.get(stream_id).await else {
            return;
        };
        if stored.branch == detected {
            return;
        }
        // Only the branch: a whole-row write could undo a rename made
        // since the read.
        if let Err(e) = self.streams.set_branch(stream_id, &detected).await {
            warn!(stream_id = %stream_id, error = %e, "failed to persist reconciled branch");
            return;
        }
        debug!(stream_id = %stream_id, branch = %detected, "reconciled stream branch from HEAD");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P5.B1 (tsk520): a checkout made outside oxplow lands on the
    /// stream's row.
    #[tokio::test]
    async fn a_checkout_outside_oxplow_updates_the_stream_branch() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let stream = svc.stream_store.list().await.unwrap().remove(0);
        let ws = svc.worktrees.resolve(Some(&stream.id.to_string())).await;
        svc.vcs
            .checkout_branch(&ws, "elsewhere", true)
            .await
            .unwrap();
        svc.branch_reconciler.reconcile(&stream.id, &ws).await;
        let row = svc.stream_store.get(&stream.id).await.unwrap().unwrap();
        assert_eq!(row.branch, "elsewhere");
        assert_eq!(row.branch_ref, "refs/heads/elsewhere");
    }

    /// P7.B6: a ref move reaches the reconciler on the VCS watcher's own
    /// channel (`RefMoves`), not the in-memory event bus.
    #[tokio::test]
    async fn a_ref_move_reaches_the_reconciler_on_its_own_channel() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.branch_reconciler.clone().spawn();
        let stream = svc.stream_store.list().await.unwrap().remove(0);
        let ws = svc.worktrees.resolve(Some(&stream.id.to_string())).await;
        svc.vcs.checkout_branch(&ws, "moved", true).await.unwrap();
        svc.ref_moves.moved(stream.id);
        for _ in 0..200 {
            let row = svc.stream_store.get(&stream.id).await.unwrap().unwrap();
            if row.branch == "moved" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the reconciler never heard the move");
    }
}
