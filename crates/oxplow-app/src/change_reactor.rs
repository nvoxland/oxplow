//! The `change.analyze` pump consumer (P7.B4): keeps the mutable changes
//! — a stream's working tree and its open efforts — analyzed as the
//! stream moves.
//!
//! On a `snapshot.taken` that recorded files (not `unchanged`) or a
//! `vcs.head.moved`, it first lists the changed files of the event's
//! stream's `working` change and each open effort's change on it (stage
//! one, [`change_analysis::refresh_files`](crate::change_analysis::refresh_files):
//! cheap, so the lists are current before anything slow runs), then
//! recomputes their deep analysis
//! ([`change_analysis::refresh_change`](crate::change_analysis::refresh_change)).
//! A burst analyzes once: an event is skipped when a newer one that would
//! trigger it for the same stream is already logged — that one recomputes.
//! On `effort.finished` it recomputes that effort's change, now against its
//! end snapshot: the effort closed before its end take, so no take event
//! reached it while it was open (tsk710; what `effort_churn`, `after:
//! [change.analyze]`, reads).
//! The results land in `v_change*` (`ModelsChanged` announces them); an
//! analysis already running for a change defers the event (`Busy`), and a
//! failure is a dead letter naming the stream.
//!
//! It holds `Services` weakly: the pump that runs it is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::{DomainError, StoredEvent};

use crate::change_analysis::ChangeTarget;
use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "change.analyze";

const SNAPSHOT_TAKEN: &str = "snapshot.taken";
const HEAD_MOVED: &str = "vcs.head.moved";
const EFFORT_FINISHED: &str = "effort.finished";
const TURN_ENDED: &str = "agent.turn.ended";

/// When a mutable change's deep analysis runs: its file list is
/// listed on every move (stage one); the deep analysis once a burst of
/// takes settles (20 s), at most every 2 minutes, and at once when HEAD
/// moves or an agent's turn ends. A finishing effort's runs at once.
pub fn deep_pacing() -> oxplow_config::collectors::Pacing {
    oxplow_config::collectors::Pacing {
        settle_secs: Some(20),
        at_most_secs: Some(120),
        idle_secs: None,
        force: vec![HEAD_MOVED.into(), TURN_ENDED.into()],
    }
}

/// The paced job of `target`'s deep analysis in `pending_run`: owner
/// `core`, id `change/working/<stream>` or `change/effort/<effort>`.
pub fn job_id(target: &ChangeTarget) -> Option<String> {
    match target {
        ChangeTarget::Working { stream_id } => Some(format!("change/working/{stream_id}")),
        ChangeTarget::Effort { effort_id } => Some(format!("change/effort/{effort_id}")),
        _ => None,
    }
}

/// The target a `change/…` job id names.
pub fn target_of(job: &str) -> Option<ChangeTarget> {
    let rest = job.strip_prefix("change/")?;
    let (kind, id) = rest.split_once('/')?;
    match kind {
        "working" => Some(ChangeTarget::Working {
            stream_id: id.to_string(),
        }),
        "effort" => Some(ChangeTarget::Effort {
            effort_id: id.to_string(),
        }),
        _ => None,
    }
}

/// The owner of core's paced jobs.
pub const CORE: &str = "core";

pub struct ChangeReactor {
    services: Weak<Services>,
}

impl ChangeReactor {
    pub fn new(services: Weak<Services>) -> Self {
        Self { services }
    }
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(ChangeReactor::new(Arc::downgrade(svc))));
}

/// Whether `event` moves its stream's mutable changes: a take that
/// recorded files, or HEAD moving.
fn moves_the_stream(event: &StoredEvent) -> bool {
    match event.envelope.event_type.as_str() {
        SNAPSHOT_TAKEN => {
            let p = &event.envelope.payload;
            !p["unchanged"].as_bool().unwrap_or(false) && p["file_count"].as_u64().unwrap_or(0) > 0
        }
        HEAD_MOVED => true,
        _ => false,
    }
}

/// Whether a newer event that moves `stream` is already logged after
/// `seq`: it recomputes, so this one needn't.
async fn superseded(svc: &Services, stream: i64, seq: i64) -> Result<bool, DomainError> {
    svc.db
        .read(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM event_log
                   WHERE seq > ?1 AND stream_id = ?2 AND payload_expired_at IS NULL
                     AND (type = 'vcs.head.moved'
                          OR (type = 'snapshot.taken'
                              AND json_extract(payload, '$.unchanged') = 0
                              AND json_extract(payload, '$.file_count') > 0)))",
                rusqlite::params![seq, stream],
                |r| r.get::<_, bool>(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
}

#[async_trait]
impl AsyncEventConsumer for ChangeReactor {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        matches!(
            event_type,
            SNAPSHOT_TAKEN | HEAD_MOVED | EFFORT_FINISHED | TURN_ENDED
        )
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        if event.envelope.event_type == EFFORT_FINISHED {
            let effort = crate::effort_lifecycle::effort_of(event)?;
            return deep_now(
                &svc,
                ChangeTarget::Effort {
                    effort_id: effort.to_string(),
                },
                event.seq,
            )
            .await
            .map_err(|e| match e {
                DomainError::Busy(_) => e,
                e => DomainError::Invalid(format!("analyzing finished {effort}: {e}")),
            });
        }
        let Some(stream) = event.envelope.anchors.stream_id else {
            return Ok(());
        };
        let turn_ended = event.envelope.event_type == TURN_ENDED;
        if !turn_ended
            && (!moves_the_stream(event) || superseded(&svc, stream.value(), event.seq).await?)
        {
            return Ok(());
        }
        let failed = |e: DomainError| match e {
            DomainError::Busy(_) => e,
            e => DomainError::Invalid(format!("analyzing stream {stream}: {e}")),
        };
        // Stage one first, for every mutable change: the file lists are
        // current before any deep analysis begins.
        let open: Vec<_> = svc
            .effort_store
            .list_open_for_stream(stream)
            .await?
            .into_iter()
            // An effort with no start snapshot has nothing to diff.
            .filter(|e| e.start_snapshot_id.is_some())
            .collect();
        let targets: Vec<ChangeTarget> = std::iter::once(ChangeTarget::Working {
            stream_id: stream.to_string(),
        })
        .chain(open.into_iter().map(|e| ChangeTarget::Effort {
            effort_id: e.id.to_string(),
        }))
        .collect();
        // A turn's end doesn't move the tree (its take does): its lists
        // are as current as the last take left them.
        if !turn_ended {
            for target in &targets {
                crate::change_analysis::refresh_files(&svc, target.clone())
                    .await
                    .map_err(failed)?;
            }
        }
        // The deep analysis: at once when this event forces it, else
        // paced (`crate::pacing` runs it once due).
        let forced = !deep_pacing().defers(&event.envelope.event_type);
        let now = oxplow_domain::Timestamp::now().to_text();
        for target in targets {
            if forced {
                deep_now(&svc, target, event.seq).await.map_err(failed)?;
            } else if let Some(job) = job_id(&target) {
                svc.collector_store
                    .mark_pending(CORE, &job, event.seq, &now)
                    .await?;
            }
        }
        Ok(())
    }
}

/// Analyze `target` now and clear its paced job: this run covers it.
async fn deep_now(svc: &Services, target: ChangeTarget, seq: i64) -> Result<(), DomainError> {
    let job = job_id(&target);
    crate::change_analysis::refresh_change(svc, target).await?;
    if let Some(job) = job {
        svc.collector_store.clear_pending(CORE, &job, seq).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{
        SnapshotTaken, SnapshotTakenV2, VcsHeadMoved, VcsHeadMovedV1,
    };
    use oxplow_domain::events::Anchors;
    use oxplow_domain::{Envelope, StreamId};

    const BEFORE: &str = "fn a() -> i32 {\n    1\n}\n";

    async fn log(svc: &Services, env: Envelope) -> StoredEvent {
        let env = env.with_anchors(Anchors {
            stream_id: Some(StreamId::new(1)),
            ..Anchors::default()
        });
        let id = env.id.clone();
        svc.event_log_store.append(env).await.unwrap();
        svc.event_log_store.get(id).await.unwrap().unwrap()
    }

    fn taken(unchanged: bool, file_count: u32) -> Envelope {
        Envelope::typed::<SnapshotTaken>(
            "system",
            &SnapshotTakenV2 {
                stream: "stream:str1".into(),
                snapshot: "snapshot:1".into(),
                parent: None,
                trigger: oxplow_domain::snapshot::SnapshotTrigger::TurnEnd,
                unchanged,
                file_count,
                elapsed_ms: 1,
                budget_ms: None,
                over_budget: false,
            },
        )
    }

    fn head_moved() -> Envelope {
        Envelope::typed::<VcsHeadMoved>(
            "system",
            &VcsHeadMovedV1 {
                stream: "stream:str1".into(),
                snapshot: "snapshot:1".into(),
                from: None,
                to: "commit:abc".into(),
            },
        )
    }

    async fn working_paths(svc: &Services) -> serde_json::Value {
        let out = svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'working' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// A repo with one committed file and an uncommitted edit.
    fn repo(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        crate::test_fixtures::commit_all(root, "base");
        std::fs::write(root.join("src/lib.rs"), "fn a() -> i32 {\n    2\n}\n").unwrap();
    }

    /// A move lists the working tree's files first, on their own:
    /// even while a deep analysis of it is still running (the deep step
    /// defers, `Busy`), the file list is current.
    #[tokio::test]
    async fn a_move_lists_the_files_even_while_the_analysis_runs() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        let id = f
            .svc
            .sql
            .query_sql(
                "SELECT id FROM v_change WHERE kind = 'working'",
                vec![],
                None,
            )
            .await
            .unwrap();
        let id = match id.rows[0][0] {
            oxplow_db::SqlCell::Int(n) => n,
            ref other => panic!("{other:?}"),
        };
        crate::change_analysis::hold_running_for_tests(&f.svc, id);
        std::fs::write(f.svc.layout.project_dir.join("src/other.rs"), "fn x() {}\n").unwrap();
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"], ["src/other.rs"]])
        );
    }

    /// A take that recorded files lists the working tree's files
    /// at once and paces the deep analysis (`pending_run`, core's job):
    /// it runs once settled, stamping what its inputs had seen.
    #[tokio::test]
    async fn a_snapshot_lists_files_and_paces_the_deep_analysis() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let ev = log(&f.svc, taken(false, 1)).await;
        assert!(reactor.handles(&ev.envelope.event_type));
        reactor.handle(&ev).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
        let deep = |svc: Arc<Services>| async move {
            let out = svc
                .sql
                .query_sql(
                    "SELECT events_to IS NOT NULL, (SELECT count(*) FROM v_pending_run WHERE owner = 'core')
                     FROM v_change WHERE kind = 'working'",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        assert_eq!(
            deep(f.svc.clone()).await,
            serde_json::json!([[0, 1]]),
            "paced"
        );
        let later = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() + 21_000,
        );
        assert_eq!(crate::pacing::run_due(&f.svc, later).await.unwrap(), 1);
        assert_eq!(
            deep(f.svc.clone()).await,
            serde_json::json!([[1, 0]]),
            "ran and cleared"
        );
    }

    /// An agent's turn ending analyzes at once — the moment a
    /// person looks at what it did — and clears the paced job.
    #[tokio::test]
    async fn a_turn_ending_analyzes_at_once() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        let ended = log(
            &f.svc,
            Envelope::typed::<oxplow_domain::events::schema::AgentTurnEnded>(
                "system",
                &oxplow_domain::events::schema::AgentTurnEndedV2 {
                    turn: "agent_turn:1".into(),
                    thread: "thread:thr1".into(),
                    outcome: oxplow_domain::hook::TurnOutcome::Completed,
                    transcript_path: None,
                    usage: None,
                },
            ),
        )
        .await;
        assert!(reactor.handles(&ended.envelope.event_type));
        reactor.handle(&ended).await.unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT events_to IS NOT NULL, (SELECT count(*) FROM v_pending_run)
                 FROM v_change WHERE kind = 'working'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.rows).unwrap(),
            serde_json::json!([[1, 0]])
        );
    }

    /// An unchanged take moves nothing.
    #[tokio::test]
    async fn an_unchanged_take_does_nothing() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(true, 0)).await)
            .await
            .unwrap();
        assert_eq!(working_paths(&f.svc).await, serde_json::json!([]));
    }

    /// A burst analyzes once: an event a newer one supersedes is skipped.
    #[tokio::test]
    async fn a_burst_computes_once() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let first = log(&f.svc, taken(false, 1)).await;
        let second = log(&f.svc, taken(false, 2)).await;
        // An unchanged take after them supersedes nothing.
        log(&f.svc, taken(true, 0)).await;
        reactor.handle(&first).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([]),
            "superseded"
        );
        reactor.handle(&second).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
    }

    /// An open effort on the stream is recomputed too — from its start
    /// snapshot to the working tree.
    #[tokio::test]
    async fn a_snapshot_recomputes_open_efforts() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            root.clone(),
            Arc::new(crate::vcs::GitProvider),
            StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        capture.mark_dirty(
            root.join("src/lib.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        let start = capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
            .await
            .unwrap()
            .unwrap();
        f.svc
            .effort_store
            .set_start_snapshot(&f.effort, start)
            .await
            .unwrap();
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'effort' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        let paths = serde_json::to_value(out.rows).unwrap();
        assert!(
            paths
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(["src/new.rs"])),
            "{paths}"
        );
    }

    /// P7 review (tsk710): an effort that edits and finishes before any take
    /// while it is open is recomputed against its end snapshot when
    /// `effort.finished` arrives — what `effort_churn` (`after:
    /// [change.analyze]`) reads.
    #[tokio::test]
    async fn a_finished_effort_is_recomputed_against_its_end_snapshot() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            root.clone(),
            Arc::new(crate::vcs::GitProvider),
            StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        let take = |path: &str| {
            capture.mark_dirty(root.join(path), oxplow_fs_watch::WatchEventKind::Other);
            capture.request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
        };
        let start = take("src/lib.rs").await.unwrap().unwrap();
        f.svc
            .effort_store
            .set_start_snapshot(&f.effort, start)
            .await
            .unwrap();
        // Analyzed once while open, before the edit.
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        // The edit lands and the effort closes: its end snapshot holds it.
        std::fs::write(root.join("src/late.rs"), "fn late() {}\n").unwrap();
        let end = take("src/late.rs").await.unwrap().unwrap();
        f.svc
            .effort_store
            .set_end_snapshot(&f.effort, end)
            .await
            .unwrap();
        let effort = f.effort;
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE effort SET ended_at = '2026-10-02T00:00:00.000000Z' WHERE id = ?1",
                    [effort.value()],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert!(reactor.handles("effort.finished"));
        let finished = Envelope::typed::<oxplow_domain::events::schema::EffortFinished>(
            "system",
            &oxplow_domain::events::schema::EffortFinishedV2 {
                // The ref, as `effort.lifecycle` logs it (tsk1025).
                effort: oxplow_domain::refs::build::effort_ref(f.effort),
                work_item: Some("work_item:oxplow:tsk1".into()),
                end_snapshot: Some(format!("snapshot:{end}")),
                retroactive: false,
            },
        );
        reactor.handle(&log(&f.svc, finished).await).await.unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'effort' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        let paths = serde_json::to_value(out.rows).unwrap();
        assert!(
            paths
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(["src/late.rs"])),
            "{paths}"
        );
    }

    /// A failed analysis is an error naming the stream (the pump makes it a
    /// dead letter).
    #[tokio::test]
    async fn a_failed_analysis_names_the_stream() {
        let f = crate::test_fixtures::services_with_effort().await;
        std::fs::remove_dir_all(f.svc.layout.project_dir.join(".git")).unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let err = reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("analyzing stream str1"), "{err}");
    }

    /// HEAD moving recomputes the working change.
    #[tokio::test]
    async fn a_head_move_recomputes_the_working_change() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, head_moved()).await)
            .await
            .unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
    }
}
