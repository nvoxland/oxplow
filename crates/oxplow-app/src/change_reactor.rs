//! The `change.analyze` pump consumer (P7.B4): keeps the mutable changes
//! — a stream's working tree and its open efforts — analyzed as the
//! stream moves.
//!
//! On a `snapshot.taken` that recorded files (not `unchanged`) or a
//! `vcs.head.moved`, it recomputes the event's stream's `working` change
//! and each open effort's change on it
//! ([`change_analysis::refresh_change`](crate::change_analysis::refresh_change)).
//! A burst analyzes once: an event is skipped when a newer one that would
//! trigger it for the same stream is already logged — that one recomputes.
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
        matches!(event_type, SNAPSHOT_TAKEN | HEAD_MOVED)
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let Some(stream) = event.envelope.anchors.stream_id else {
            return Ok(());
        };
        if !moves_the_stream(event) || superseded(&svc, stream.value(), event.seq).await? {
            return Ok(());
        }
        let failed = |e: DomainError| match e {
            DomainError::Busy(_) => e,
            e => DomainError::Invalid(format!("analyzing stream {stream}: {e}")),
        };
        crate::change_analysis::refresh_change(
            &svc,
            ChangeTarget::Working {
                stream_id: stream.to_string(),
            },
        )
        .await
        .map_err(failed)?;
        for effort in svc.effort_store.list_open_for_stream(stream).await? {
            // An effort with no start snapshot has nothing to diff.
            if effort.start_snapshot_id.is_none() {
                continue;
            }
            crate::change_analysis::refresh_change(
                &svc,
                ChangeTarget::Effort {
                    effort_id: effort.id.to_string(),
                },
            )
            .await
            .map_err(failed)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{
        SnapshotTaken, SnapshotTakenV1, VcsHeadMoved, VcsHeadMovedV1,
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
            &SnapshotTakenV1 {
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

    /// A take that recorded files recomputes the stream's working change,
    /// stamping it with how far into the log its inputs were.
    #[tokio::test]
    async fn a_snapshot_recomputes_the_working_change() {
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

        std::fs::write(f.svc.layout.project_dir.join("src/other.rs"), "fn x() {}\n").unwrap();
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"], ["src/other.rs"]])
        );
        let row = f
            .svc
            .sql
            .query_sql(
                "SELECT events_to FROM v_change WHERE kind = 'working'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert!(
            row.rows[0][0] != oxplow_db::SqlCell::Null(()),
            "events_to is stamped"
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
