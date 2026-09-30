//! The snapshot capability's contract (P2.8, tsk432;
//! `.context/target-architecture.md` §6.1), as tests run against the one
//! provider there is — `SnapshotCaptureService` over the local worktree.
//! A second provider (a remote worktree, a VCS-native one) must pass the
//! same six properties; the `SnapshotProvider` trait waits until one
//! exists, and these tests are its specification:
//!
//! 1. **Round-trip** — what a take captured reads back byte-for-byte.
//! 2. **Content identity** — a take that finds the tree unchanged returns
//!    the snapshot it already has, not a new one.
//! 3. **Diffs** — two snapshots diff to what changed between them.
//! 4. **Deletions** — a removed file is gone from the next snapshot.
//! 5. **Per-turn anchoring** — a turn-end take is anchored to its turn.
//! 6. **Budget** — a take given a budget records it, and records when it
//!    ran over (it never aborts: a snapshot is never cut short).

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxplow_domain::snapshot::SnapshotTrigger;
    use oxplow_domain::{ChangeStatus, StreamId};
    use oxplow_fs_watch::WatchEventKind;

    use crate::snapshot_capture::{SnapshotCaptureService, TakeRequest};
    use crate::test_fixtures::{services_with_effort, EffortFixture};

    /// A capture service over the fixture's worktree, with the settle and
    /// predrain gates off so a take sees what was just written.
    fn provider(f: &EffortFixture, predrain: Duration) -> (SnapshotCaptureService, StreamId) {
        let stream = StreamId::new(1);
        let svc = SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            f.svc.layout.project_dir.clone(),
            std::sync::Arc::new(crate::vcs::GitProvider),
            stream,
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(Duration::ZERO)
        .with_predrain_delay(predrain);
        (svc, stream)
    }

    fn write(f: &EffortFixture, svc: &SnapshotCaptureService, path: &str, body: &str) {
        let full = f.svc.layout.project_dir.join(path);
        std::fs::write(&full, body).unwrap();
        svc.mark_dirty(full, WatchEventKind::Other);
    }

    fn remove(f: &EffortFixture, svc: &SnapshotCaptureService, path: &str) {
        let full = f.svc.layout.project_dir.join(path);
        std::fs::remove_file(&full).unwrap();
        svc.mark_dirty(full, WatchEventKind::Other);
    }

    async fn take(svc: &SnapshotCaptureService, req: TakeRequest) -> i64 {
        svc.request_snapshot(req)
            .await
            .unwrap()
            .expect("the take has a snapshot")
    }

    fn manual() -> TakeRequest {
        TakeRequest {
            trigger: SnapshotTrigger::Manual,
            thread_id: None,
            turn_id: None,
            effort_id: None,
            budget: None,
        }
    }

    async fn read(svc: &SnapshotCaptureService, snapshot: i64, path: &str) -> Option<Vec<u8>> {
        let content = svc
            .store()
            .content_ref_for_path(snapshot, path)
            .await
            .unwrap()?;
        Some(svc.content().read_ref(&content).unwrap())
    }

    #[tokio::test]
    async fn a_take_reads_back_what_it_captured() {
        let f = services_with_effort().await;
        let (svc, _) = provider(&f, Duration::ZERO);
        write(&f, &svc, "a.txt", "alpha");
        let s = take(&svc, manual()).await;
        assert_eq!(read(&svc, s, "a.txt").await.as_deref(), Some(&b"alpha"[..]));
        // Later writes don't reach back into an earlier snapshot.
        write(&f, &svc, "a.txt", "beta");
        let s2 = take(&svc, manual()).await;
        assert_eq!(read(&svc, s, "a.txt").await.as_deref(), Some(&b"alpha"[..]));
        assert_eq!(read(&svc, s2, "a.txt").await.as_deref(), Some(&b"beta"[..]));
    }

    #[tokio::test]
    async fn an_unchanged_tree_takes_no_new_snapshot() {
        let f = services_with_effort().await;
        let (svc, _) = provider(&f, Duration::ZERO);
        write(&f, &svc, "a.txt", "alpha");
        let s = take(&svc, manual()).await;
        // Rewriting the same bytes, or taking with nothing dirty.
        write(&f, &svc, "a.txt", "alpha");
        assert_eq!(take(&svc, manual()).await, s);
        assert_eq!(take(&svc, manual()).await, s);
    }

    #[tokio::test]
    async fn two_snapshots_diff_to_what_changed_including_deletions() {
        let f = services_with_effort().await;
        let (svc, _) = provider(&f, Duration::ZERO);
        write(&f, &svc, "kept.txt", "k");
        write(&f, &svc, "edited.txt", "v1");
        write(&f, &svc, "gone.txt", "g");
        let before = take(&svc, manual()).await;
        write(&f, &svc, "edited.txt", "v2");
        write(&f, &svc, "new.txt", "n");
        remove(&f, &svc, "gone.txt");
        let after = take(&svc, manual()).await;

        let changes: Vec<(String, ChangeStatus)> = svc
            .store()
            .diff_snapshots(Some(before), after)
            .await
            .unwrap()
            .into_iter()
            .map(|c| (c.path, c.status))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("edited.txt".to_string(), ChangeStatus::Modified),
                ("gone.txt".to_string(), ChangeStatus::Deleted),
                ("new.txt".to_string(), ChangeStatus::Added),
            ]
        );
        // The deletion holds in the snapshot itself, not just the diff.
        assert_eq!(read(&svc, after, "gone.txt").await, None);
        assert!(!svc
            .store()
            .tree_at(after)
            .await
            .unwrap()
            .contains_key("gone.txt"));
    }

    #[tokio::test]
    async fn a_turn_end_take_is_anchored_to_its_turn() {
        let f = services_with_effort().await;
        let (svc, stream) = provider(&f, Duration::ZERO);
        let turn: i64 = f
            .svc
            .db
            .transaction(|c| {
                c.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, started_at)
                     VALUES (1, 'p', '2026-01-01T00:00:00.000000Z')",
                    [],
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();
        write(&f, &svc, "a.txt", "turn work");
        let s = take(
            &svc,
            TakeRequest {
                trigger: SnapshotTrigger::TurnEnd,
                thread_id: Some(f.thread),
                turn_id: Some(turn),
                effort_id: Some(f.effort),
                budget: None,
            },
        )
        .await;
        let op = svc.store().list_ops(stream, 1).await.unwrap().remove(0);
        assert_eq!(op.snapshot_id, s);
        assert_eq!(op.trigger, SnapshotTrigger::TurnEnd);
        assert_eq!(op.turn_id, Some(turn));
        assert_eq!(op.thread_id, Some(f.thread));
        assert_eq!(op.effort_id, Some(f.effort));
    }

    #[tokio::test]
    async fn a_take_records_its_budget_and_whether_it_ran_over() {
        let f = services_with_effort().await;
        let budgeted = |ms: u64| TakeRequest {
            trigger: SnapshotTrigger::TurnEnd,
            thread_id: Some(f.thread),
            turn_id: None,
            effort_id: None,
            budget: Some(Duration::from_millis(ms)),
        };
        let (fast, stream) = provider(&f, Duration::ZERO);
        write(&f, &fast, "a.txt", "quick");
        take(&fast, budgeted(10_000)).await;
        let op = fast.store().list_ops(stream, 1).await.unwrap().remove(0);
        assert_eq!((op.budget_ms, op.over_budget), (Some(10_000), false));

        // A take that can't finish inside its budget still finishes, and
        // says so.
        let (slow, _) = provider(&f, Duration::from_millis(300));
        write(&f, &slow, "b.txt", "slow");
        let s = take(&slow, budgeted(50)).await;
        let op = slow.store().list_ops(stream, 1).await.unwrap().remove(0);
        assert_eq!(op.snapshot_id, s);
        assert_eq!((op.budget_ms, op.over_budget), (Some(50), true));
        assert!(op.elapsed_ms >= 50);
    }
}
