//! The snapshot capability's contract (P2.8;
//! `.context/data-model.md`), as a suite run through the interface —
//! [`SnapshotProvider`] — so the built-ins and any provider that
//! implements it are held to the same properties:
//!
//! 1. **Round-trip** — what a mark captured reads back byte-for-byte, and
//!    a later write doesn't reach back into it. An implementation that
//!    keeps no contents (`contents()` false) answers the read with
//!    `NoContents`; that answer is the check.
//! 2. **Content identity** — a mark that finds the tree unchanged returns
//!    the snapshot it already has, not a new one.
//! 3. **Diffs** — two snapshots differ by what was added, modified and
//!    deleted between them, however little of it was kept.
//! 4. **Per-turn anchoring** — a turn-end mark is anchored to its turn.
//! 5. **Budget** — a mark given a budget records it, and says whether it
//!    ran over (it never aborts: a snapshot is never cut short).
//!
//! The suite writes a few files under the stream's worktree (and removes
//! them) and tells the stream's capture service about them, standing in
//! for the file watcher; it records marks on the stream, so run it on a
//! scratch stream or accept the extra snapshots.

use std::time::Duration;

use oxplow_domain::snapshot::{
    MarkRequest, Marked, SnapshotError, SnapshotProvider, SnapshotTrigger,
};
use oxplow_domain::tree_diff::ChangeStatus;
use oxplow_domain::StreamId;
use oxplow_fs_watch::WatchEventKind;

pub use crate::work_items_conformance::{Finding, SuiteRun};

/// The directory (under the worktree) the suite's files live in.
const SCRATCH: &str = "conformance-snapshots";

/// Run every check against `provider`, on `stream`.
pub async fn suite(
    svc: &crate::Services,
    provider: &dyn SnapshotProvider,
    stream: StreamId,
) -> SuiteRun {
    let mut findings = Vec::new();
    match svc.snapshot_captures.get(&stream) {
        None => findings.push(Finding {
            check: "setup",
            message: format!("stream {stream} has no capture service to write scratch files in"),
        }),
        Some(capture) => {
            let dir = capture.project_dir().join(SCRATCH);
            let ctx = Ctx {
                svc,
                provider,
                stream,
                capture: &capture,
                dir: dir.clone(),
            };
            let _ = std::fs::create_dir_all(&dir);
            ctx.run(&mut findings).await;
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    SuiteRun {
        findings,
        left: Vec::new(),
    }
}

struct Ctx<'a> {
    svc: &'a crate::Services,
    provider: &'a dyn SnapshotProvider,
    stream: StreamId,
    capture: &'a crate::snapshot_capture::SnapshotCaptureService,
    dir: std::path::PathBuf,
}

impl Ctx<'_> {
    /// The workspace-relative name of scratch file `name`.
    fn rel(name: &str) -> String {
        format!("{SCRATCH}/{name}")
    }

    fn write(&self, name: &str, body: &str) {
        let full = self.dir.join(name);
        let _ = std::fs::write(&full, body);
        self.capture.mark_dirty(full, WatchEventKind::Other);
    }

    fn remove(&self, name: &str) {
        let full = self.dir.join(name);
        let _ = std::fs::remove_file(&full);
        self.capture.mark_dirty(full, WatchEventKind::Other);
    }

    async fn mark(&self, request: MarkRequest) -> Result<Marked, String> {
        match self.provider.mark(&request).await {
            Ok(Some(marked)) => Ok(marked),
            Ok(None) => Err("a mark of the stream recorded nothing".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn plain(&self) -> MarkRequest {
        MarkRequest::new(self.stream, SnapshotTrigger::Manual)
    }

    async fn run(&self, findings: &mut Vec<Finding>) {
        let mut fail = |check: &'static str, message: String| {
            findings.push(Finding { check, message });
        };

        // 1. Round-trip, and 2. content identity.
        let (a, b) = (Self::rel("a.txt"), Self::rel("b.txt"));
        self.write("a.txt", "alpha");
        let first = match self.mark(self.plain()).await {
            Ok(m) => m,
            Err(e) => {
                fail("a_mark_records_a_snapshot", e);
                return;
            }
        };
        self.write("a.txt", "beta");
        let second = match self.mark(self.plain()).await {
            Ok(m) => m,
            Err(e) => {
                fail("a_mark_records_a_snapshot", e);
                return;
            }
        };
        if second.snapshot == first.snapshot {
            fail(
                "a_mark_of_a_changed_tree_is_a_new_snapshot",
                format!("a changed file marked as snapshot {} again", first.snapshot),
            );
        }
        let read = |snapshot: i64| self.provider.read_at(snapshot, &a);
        if self.provider.contents() {
            for (snapshot, want) in [(first.snapshot, "alpha"), (second.snapshot, "beta")] {
                match read(snapshot).await {
                    Ok(bytes) if bytes == want.as_bytes() => {}
                    other => fail(
                        "a_mark_reads_back_what_it_captured",
                        format!("at snapshot {snapshot} {a} read as {other:?}, not `{want}`"),
                    ),
                }
            }
        } else {
            match read(first.snapshot).await {
                Err(SnapshotError::NoContents { .. }) => {}
                other => fail(
                    "an_implementation_without_contents_answers_no_contents",
                    format!("it keeps no contents, yet reading {a} gave {other:?}, not NoContents"),
                ),
            }
        }

        // Nothing changed since: the same snapshot, flagged unchanged.
        for _ in 0..2 {
            match self.mark(self.plain()).await {
                Ok(m) if m.snapshot == second.snapshot && m.unchanged => {}
                Ok(m) => fail(
                    "an_unchanged_tree_marks_the_snapshot_it_has",
                    format!(
                        "an unchanged tree marked as snapshot {} (unchanged: {}), not {}",
                        m.snapshot, m.unchanged, second.snapshot
                    ),
                ),
                Err(e) => fail("an_unchanged_tree_marks_the_snapshot_it_has", e),
            }
        }
        // Rewriting the same bytes is no change either.
        self.write("a.txt", "beta");
        match self.mark(self.plain()).await {
            Ok(m) if m.snapshot == second.snapshot => {}
            Ok(m) => fail(
                "an_unchanged_tree_marks_the_snapshot_it_has",
                format!(
                    "rewriting a file with the same bytes marked snapshot {}, not {}",
                    m.snapshot, second.snapshot
                ),
            ),
            Err(e) => fail("an_unchanged_tree_marks_the_snapshot_it_has", e),
        }

        // 3. Diffs: added, modified, deleted.
        let (c, d) = (Self::rel("c.txt"), Self::rel("d.txt"));
        self.write("b.txt", "keep");
        self.write("c.txt", "v1");
        self.write("d.txt", "bye");
        let before = self.mark(self.plain()).await;
        self.write("c.txt", "v2");
        self.write("e.txt", "new");
        self.remove("d.txt");
        let after = self.mark(self.plain()).await;
        match (before, after) {
            (Ok(before), Ok(after)) => {
                match self
                    .provider
                    .changed(self.stream, Some(before.snapshot), after.snapshot)
                    .await
                {
                    Ok(changes) => {
                        let mut got: Vec<(String, ChangeStatus)> =
                            changes.into_iter().map(|c| (c.path, c.status)).collect();
                        got.sort_by(|x, y| x.0.cmp(&y.0));
                        let want = vec![
                            (c.clone(), ChangeStatus::Modified),
                            (d.clone(), ChangeStatus::Deleted),
                            (Self::rel("e.txt"), ChangeStatus::Added),
                        ];
                        if got != want {
                            fail(
                                "changed_names_what_was_added_modified_and_deleted",
                                format!("changed gave {got:?}, not {want:?} ({b} untouched)"),
                            );
                        }
                    }
                    Err(e) => fail(
                        "changed_names_what_was_added_modified_and_deleted",
                        e.to_string(),
                    ),
                }
            }
            (Err(e), _) | (_, Err(e)) => fail("a_mark_records_a_snapshot", e),
        }

        // 4. A turn-end mark is anchored to its turn.
        match self.turn().await {
            None => {}
            Some((thread, turn)) => {
                self.write("turn.txt", "turn work");
                let mut request = MarkRequest::new(self.stream, SnapshotTrigger::TurnEnd);
                request.thread = Some(thread);
                request.turn = Some(turn);
                match self.mark(request).await {
                    Ok(m) => match self.svc.snapshot_store.list_ops(self.stream, 1).await {
                        Ok(ops) => match ops.first() {
                            Some(op)
                                if op.snapshot_id == m.snapshot
                                    && op.trigger == SnapshotTrigger::TurnEnd
                                    && op.turn_id == Some(turn)
                                    && op.thread_id == Some(thread) => {}
                            other => fail(
                                "a_turn_end_mark_is_anchored_to_its_turn",
                                format!("the newest op is {other:?}, not turn {turn} ending at snapshot {}", m.snapshot),
                            ),
                        },
                        Err(e) => fail("a_turn_end_mark_is_anchored_to_its_turn", e.to_string()),
                    },
                    Err(e) => fail("a_turn_end_mark_is_anchored_to_its_turn", e),
                }
            }
        }

        // 5. A budget is recorded, and its overrun flagged honestly.
        for (name, ms) in [("fast.txt", 600_000u64), ("tight.txt", 1)] {
            self.write(name, "budget");
            let mut request = self.plain();
            request.budget = Some(Duration::from_millis(ms));
            match self.mark(request).await {
                Ok(m) => match self.svc.snapshot_store.list_ops(self.stream, 1).await {
                    Ok(ops) => match ops.first() {
                        Some(op)
                            if op.budget_ms == Some(ms as i64)
                                && op.over_budget == (op.elapsed_ms > ms as i64)
                                && m.over_budget == op.over_budget => {}
                        other => fail(
                            "a_mark_records_its_budget_and_whether_it_ran_over",
                            format!("with a {ms} ms budget the newest op is {other:?} (marked over_budget: {})", m.over_budget),
                        ),
                    },
                    Err(e) => fail("a_mark_records_its_budget_and_whether_it_ran_over", e.to_string()),
                },
                Err(e) => fail("a_mark_records_its_budget_and_whether_it_ran_over", e),
            }
        }
    }

    /// A thread of the stream and a turn of it to end (a scratch turn row).
    async fn turn(&self) -> Option<(oxplow_domain::ThreadId, i64)> {
        let stream = self.stream.value();
        self.svc
            .db
            .transaction(move |c| {
                use rusqlite::OptionalExtension as _;
                let thread: Option<i64> = c
                    .query_row(
                        "SELECT id FROM threads WHERE stream_id = ?1 ORDER BY id LIMIT 1",
                        [stream],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(oxplow_db::map_sql_err)?;
                let Some(thread) = thread else {
                    return Ok(None);
                };
                c.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, started_at)
                     VALUES (?1, 'snapshot conformance', '2026-01-01T00:00:00.000000Z')",
                    [thread],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(Some((
                    oxplow_domain::ThreadId::new(thread),
                    c.last_insert_rowid(),
                )))
            })
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use oxplow_domain::snapshot::{MarkRequest, SnapshotTrigger};
    use oxplow_domain::StreamId;

    use super::*;
    use crate::snapshot_capture::SnapshotCaptureService;
    use crate::test_fixtures::{services_with_effort, EffortFixture};

    /// The fixture's stream with a capture service whose settle gate is
    /// off and whose predrain wait is `predrain`.
    fn stream_of(f: &EffortFixture, predrain: Duration) -> StreamId {
        let stream = StreamId::new(1);
        let svc = Arc::new(
            SnapshotCaptureService::new(
                f.svc.snapshot_store.clone(),
                f.svc.blobs.clone(),
                f.svc.layout.project_dir.clone(),
                Arc::new(crate::vcs::GitProvider),
                stream,
                1_000_000,
                oxplow_fs_watch::WorkspaceFilter::default(),
            )
            .with_settle_duration(Duration::ZERO)
            .with_predrain_delay(predrain),
        );
        f.svc.snapshot_captures.insert_for_test(stream, svc);
        stream
    }

    /// `id` as the project's snapshot implementation, its switch applied.
    async fn choose(f: &EffortFixture, id: &str) {
        f.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(crate::snapshots::CAPABILITY.into(), id.into());
        let config = crate::config_service::read_config(&f.svc.config);
        f.svc
            .capabilities
            .publish(&config, &f.svc.db)
            .await
            .unwrap();
        f.svc
            .event_pump
            .settle(&["snapshots.switch"], Duration::from_secs(10))
            .await;
    }

    /// Both snapshot built-ins keep the contract: "Keep every version"
    /// reads everything back, "Track changes only" answers `NoContents`,
    /// and both mark, dedupe, diff, anchor and budget the same.
    #[tokio::test]
    async fn every_built_in_snapshot_implementation_passes() {
        for id in ["oxplow", "hashes"] {
            let f = services_with_effort().await;
            let stream = stream_of(&f, Duration::ZERO);
            choose(&f, id).await;
            let active = f.svc.snapshots.active().expect("an active implementation");
            assert_eq!(active.id(), id);
            let run = suite(&f.svc, &*active, stream).await;
            assert!(run.findings.is_empty(), "{id}: {:#?}", run.findings);
        }
    }

    /// A mark that can't finish inside its budget still finishes, and says
    /// so, through the interface.
    #[tokio::test]
    async fn a_mark_that_ran_over_its_budget_is_flagged() {
        let f = services_with_effort().await;
        let stream = stream_of(&f, Duration::from_millis(300));
        let active = f.svc.snapshots.active().unwrap();
        std::fs::write(f.svc.layout.project_dir.join("slow.txt"), "slow").unwrap();
        f.svc.snapshot_captures.get(&stream).unwrap().mark_dirty(
            f.svc.layout.project_dir.join("slow.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        let mut request = MarkRequest::new(stream, SnapshotTrigger::Manual);
        request.budget = Some(Duration::from_millis(50));
        let marked = active.mark(&request).await.unwrap().unwrap();
        assert!(marked.over_budget);
        let op = f
            .svc
            .snapshot_store
            .list_ops(stream, 1)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(op.snapshot_id, marked.snapshot);
        assert_eq!((op.budget_ms, op.over_budget), (Some(50), true));
    }

    /// A provider off the contract is named check by check: one that
    /// answers every read with the same bytes and every diff with nothing.
    #[tokio::test]
    async fn an_implementation_off_the_contract_is_named_check_by_check() {
        use oxplow_domain::snapshot::SnapshotError;
        use oxplow_domain::tree_diff::FileChange;

        struct Sloppy {
            marks: std::sync::atomic::AtomicI64,
        }
        #[async_trait::async_trait]
        impl SnapshotProvider for Sloppy {
            fn id(&self) -> &str {
                "sloppy"
            }
            fn contents(&self) -> bool {
                true
            }
            async fn mark(&self, _: &MarkRequest) -> Result<Option<Marked>, SnapshotError> {
                let n = self.marks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(Some(Marked {
                    snapshot: n + 1,
                    parent: None,
                    unchanged: false,
                    file_count: 1,
                    over_budget: false,
                }))
            }
            async fn changed(
                &self,
                _: StreamId,
                _: Option<i64>,
                _: i64,
            ) -> Result<Vec<FileChange>, SnapshotError> {
                Ok(Vec::new())
            }
            async fn read_at(&self, _: i64, _: &str) -> Result<Vec<u8>, SnapshotError> {
                Ok(b"same".to_vec())
            }
        }

        let f = services_with_effort().await;
        let stream = stream_of(&f, Duration::ZERO);
        let run = suite(
            &f.svc,
            &Sloppy {
                marks: Default::default(),
            },
            stream,
        )
        .await;
        let mut checks: Vec<&str> = run.findings.iter().map(|f| f.check).collect();
        checks.dedup();
        assert!(
            checks.contains(&"a_mark_reads_back_what_it_captured")
                && checks.contains(&"an_unchanged_tree_marks_the_snapshot_it_has")
                && checks.contains(&"changed_names_what_was_added_modified_and_deleted"),
            "{:#?}",
            run.findings
        );
    }
}
