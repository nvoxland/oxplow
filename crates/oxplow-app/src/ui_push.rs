//! The UI push layer (P7.B6, `.context/ipc-and-stores.md`): the in-memory
//! [`EventBus`] is the renderer's channel and nothing else. What it
//! carries comes from three places — a model's tables committing
//! (`ModelsChanged`, `models_changed.rs`), the event log (this module's
//! `ui.push` consumer), and the few UI-only signals with no durable fact
//! behind them (a toast, a stream orphaned, an agent's status). Backend
//! work listens to none of it: it runs on the event pump, an asset, the
//! VCS watcher's ref moves or the extension catalog's signal.
//! `source_guards::the_bus_has_one_listener` holds that: only the `/events` forwarder
//! subscribes (`EventBus::subscribe_ui`), outside tests.

use oxplow_domain::{DomainError, StoredEvent, StreamId};

use crate::event_pump::EventConsumer;
use crate::events::{EventBus, OxplowEvent};

pub const NAME: &str = "ui.push";

/// The log's facts the renderer hears as UI events.
pub struct UiPush {
    pub events: EventBus,
}

impl EventConsumer for UiPush {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        matches!(event_type, "snapshot.taken" | "vcs.head.moved")
    }

    /// A take that recorded something new, or a clean worktree's snapshot
    /// re-stamped with a new HEAD, is `SnapshotTaken`.
    fn handle(&self, _conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let env = &event.envelope;
        let payload = &env.payload;
        let head_moved = env.event_type == "vcs.head.moved";
        // An unchanged take recorded nothing new: nothing to show.
        if payload["unchanged"].as_bool().unwrap_or(false) {
            return Ok(());
        }
        let stream = env.anchors.stream_id.or_else(|| {
            payload["stream"]
                .as_str()
                .and_then(|r| r.strip_prefix("stream:"))
                .and_then(StreamId::try_from_str)
        });
        let snapshot = env.anchors.snapshot_id.or_else(|| {
            payload["snapshot"]
                .as_str()
                .and_then(|r| r.strip_prefix("snapshot:"))
                .and_then(|n| n.parse().ok())
        });
        let trigger = if head_moved {
            oxplow_domain::snapshot::SnapshotTrigger::HeadMoved
        } else {
            serde_json::from_value(payload["trigger"].clone())
                .map_err(|e| DomainError::Invalid(format!("snapshot.taken trigger: {e}")))?
        };
        if let (Some(stream_id), Some(snapshot_id)) = (stream, snapshot) {
            self.events.emit(OxplowEvent::SnapshotTaken {
                stream_id,
                snapshot_id,
                file_count: payload["file_count"].as_u64().unwrap_or(0) as u32,
                trigger,
                thread_id: env.anchors.thread_id,
                turn_id: env.anchors.turn_id,
                effort_id: env.anchors.effort_id,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use oxplow_domain::events::schema::{SnapshotTaken, SnapshotTakenV1};
    use oxplow_domain::{Anchors, Envelope};

    use super::*;

    fn taken(stream: StreamId, snapshot: i64, unchanged: bool) -> Envelope {
        Envelope::typed::<SnapshotTaken>(
            "system:snapshots",
            &SnapshotTakenV1 {
                stream: format!("stream:{stream}"),
                snapshot: format!("snapshot:{snapshot}"),
                parent: None,
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Quiet,
                unchanged,
                file_count: if unchanged { 0 } else { 2 },
                elapsed_ms: 1,
                budget_ms: None,
                over_budget: false,
            },
        )
        .with_anchors(Anchors {
            stream_id: Some(stream),
            snapshot_id: Some(snapshot),
            ..Anchors::default()
        })
    }

    /// P7.B6: a new snapshot in the log reaches the renderer as
    /// `SnapshotTaken` through `ui.push`; an unchanged take doesn't.
    #[tokio::test]
    async fn a_logged_snapshot_reaches_the_ui() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let mut ui = svc.events.subscribe_ui();
        use oxplow_domain::stores::StreamStore as _;
        let stream = svc.stream_store.list().await.unwrap().remove(0).id;
        let vocabulary = oxplow_domain::vocabulary::VocabularyHandle::core();
        for env in [taken(stream, 7, false), taken(stream, 7, true)] {
            let vocabulary = vocabulary.clone();
            svc.db
                .transaction(move |tx| {
                    oxplow_db::event_log_store::append_tx(tx, &vocabulary.current(), &env)
                        .map(|_| ())
                })
                .await
                .unwrap();
        }
        svc.event_pump.run_once().await.unwrap();
        let mut seen = Vec::new();
        while let Ok(e) = ui.try_recv() {
            if let OxplowEvent::SnapshotTaken {
                snapshot_id,
                file_count,
                ..
            } = e
            {
                seen.push((snapshot_id, file_count));
            }
        }
        assert_eq!(seen, vec![(7, 2)]);
    }
}
