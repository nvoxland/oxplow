//! The work-items capability in the app (`.context/work-items.md`):
//!
//! - [`oxplow_provider`], the built-in provider — oxplow's own tasks —
//!   whose verbs are the `work_item.*` commands' `Tx` cores;
//! - [`WorkItems`], a typed client over the `work_item.*` commands: the
//!   one write surface for every provider (the conformance suite and
//!   `task_writes` use it);
//! - [`WorkItemsProjection`], the pump consumer (`work_items.project`)
//!   that upserts another provider's items into `work_item` from its
//!   `work_item.recorded` events. oxplow's own rows never take this path:
//!   the task cores write them with the task.

use std::sync::Arc;

use oxplow_domain::events::schema::{EventType, WorkItemRecorded, WorkItemRecordedV1};
use oxplow_domain::work_items::{
    provider_of, CanonicalState, WorkItemsFeatures, WorkItemsProvider,
};
use oxplow_domain::{Actor, CommandError, CommandOutcome, DomainError, StoredEvent};
use serde_json::{json, Value};

use crate::commands::{work_item, CommandBus};
use crate::event_pump::EventConsumer;

/// oxplow's own provider name: `work_item:oxplow:tsk<n>`.
pub const PROVIDER: &str = oxplow_domain::work_items::OXPLOW;

/// oxplow's tasks as a work-items provider: every feature, and its
/// effort follows its status. No external verbs — the `work_item.*`
/// commands run its cores in the bus's transaction.
pub fn oxplow_provider() -> WorkItemsProvider {
    WorkItemsProvider {
        id: PROVIDER.into(),
        features: WorkItemsFeatures {
            hierarchy: true,
            comments: true,
            links: true,
            delete: true,
            // Its writes run in the bus's transaction: never sent twice.
            idempotent_writes: false,
        },
        external: None,
        // Its ids: `tsk12`.
        id_pattern: Some(r"tsk\d+".into()),
    }
}

/// A new item, as `work_item.create` takes it.
#[derive(Debug, Clone, Default)]
pub struct NewItem {
    pub title: String,
    pub body: String,
    pub parent_ref: Option<String>,
    pub state: Option<CanonicalState>,
    pub native_state: Option<String>,
    pub native: Option<Value>,
}

/// The `work_item.*` commands, typed: each call is one run through the
/// bus as `actor` — dispatched to the item's provider, audited and
/// policy-checked like any other.
#[derive(Clone)]
pub struct WorkItems {
    bus: Arc<CommandBus>,
}

impl WorkItems {
    pub fn new(bus: Arc<CommandBus>) -> Self {
        Self { bus }
    }

    async fn run(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        self.bus.run(actor, name, input, false).await
    }

    /// File an item; its ref.
    pub async fn create(&self, actor: &Actor, item: NewItem) -> Result<String, CommandError> {
        let input = serde_json::to_value(work_item::WorkItemCreateInput {
            title: item.title,
            body: (!item.body.is_empty()).then_some(item.body),
            parent_ref: item.parent_ref,
            state: item.state,
            native_state: item.native_state,
            native: item.native,
            thread: None,
        })
        .expect("input serializes");
        let out = self.run(actor, work_item::CREATE, input).await?;
        out.result["ref"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| CommandError::Failed {
                message: "work_item.create returned no ref".into(),
            })
    }

    pub async fn update(
        &self,
        actor: &Actor,
        input: work_item::WorkItemUpdateInput,
    ) -> Result<CommandOutcome, CommandError> {
        let input = serde_json::to_value(input).expect("input serializes");
        self.run(actor, work_item::UPDATE, input).await
    }

    pub async fn transition(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: CanonicalState,
        native_state: Option<&str>,
    ) -> Result<CommandOutcome, CommandError> {
        let mut input = json!({ "ref": item_ref, "to": to });
        if let Some(n) = native_state {
            input["native_state"] = n.into();
        }
        self.run(actor, work_item::NAME, input).await
    }

    pub async fn link(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::LINK,
            json!({ "ref": from, "target": to, "link_type": link_type }),
        )
        .await
    }

    pub async fn comment(
        &self,
        actor: &Actor,
        item_ref: &str,
        body: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::COMMENT,
            json!({ "ref": item_ref, "body": body }),
        )
        .await
    }

    /// Destructive: `confirmed` is the person's confirmation (an agent's
    /// is ignored — its run is proposed).
    pub async fn delete(
        &self,
        actor: &Actor,
        item_ref: &str,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        self.bus
            .run(
                actor,
                work_item::DELETE,
                json!({ "ref": item_ref }),
                confirmed,
            )
            .await
    }

    /// The undo of a run (its audit row).
    pub async fn undo(&self, actor: &Actor, audit_id: i64) -> Result<CommandOutcome, CommandError> {
        self.bus.undo(actor, audit_id, false).await
    }
}

/// Upserts another provider's item into `work_item` from its
/// `work_item.recorded` event, by ref — idempotent, so a replay restates
/// the same row. An `oxplow` record is refused: those rows are the task
/// cores' alone.
pub struct WorkItemsProjection;

impl WorkItemsProjection {
    pub const NAME: &'static str = "work_items.project";
}

impl EventConsumer for WorkItemsProjection {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemRecorded::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let WorkItemRecordedV1 { item } = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("work_item.recorded payload: {e}")))?;
        let provider = provider_of(&item.item_ref)
            .map_err(|e| DomainError::Invalid(e.to_string()))?
            .to_string();
        if provider == PROVIDER {
            return Err(DomainError::Invalid(format!(
                "`{}` is oxplow's own; its row is written with the task",
                item.item_ref
            )));
        }
        let at = event.envelope.at.to_string();
        conn.execute(
            "INSERT INTO work_item (ref, provider, title, body, state, native_state, native,
                                    parent_ref, created_at, updated_at, deleted_at,
                                    filed_in_thread)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, CASE WHEN ?10 THEN ?9 END, ?11)
             ON CONFLICT(ref) DO UPDATE SET
                title = excluded.title, body = excluded.body, state = excluded.state,
                native_state = excluded.native_state, native = excluded.native,
                parent_ref = excluded.parent_ref, updated_at = excluded.updated_at,
                deleted_at = CASE WHEN ?10 THEN coalesce(work_item.deleted_at, ?9) END",
            rusqlite::params![
                item.item_ref,
                provider,
                item.title,
                item.body,
                item.state.as_str(),
                item.native_state,
                item.native.to_string(),
                item.parent_ref,
                at,
                item.deleted,
                // The thread that filed it, at its first record only
                // (tsk1041): a restatement keeps it.
                event.envelope.anchors.thread_id.map(|t| t.value()),
            ],
        )
        .map_err(|e| DomainError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::work_items::WorkItemRecord;
    use oxplow_domain::Envelope;

    fn recorded(item_ref: &str, title: &str, deleted: bool) -> Envelope {
        Envelope::typed::<WorkItemRecorded>(
            "provider:fake",
            &WorkItemRecordedV1 {
                item: WorkItemRecord {
                    item_ref: item_ref.into(),
                    title: title.into(),
                    body: String::new(),
                    state: CanonicalState::InProgress,
                    native_state: "Doing".into(),
                    native: json!({ "points": 3 }),
                    parent_ref: None,
                    deleted,
                },
            },
        )
        .with_subject([item_ref])
    }

    async fn row(svc: &crate::Services, item_ref: &str) -> Option<(String, String, bool)> {
        let item_ref = item_ref.to_string();
        svc.db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT title, state, deleted_at IS NOT NULL FROM work_item WHERE ref = ?1",
                    [item_ref],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap()
    }

    /// Another provider's items reach `work_item` from its
    /// `work_item.recorded` events, restated by ref; an oxplow record is
    /// refused (dead-lettered) — those rows are the task cores'.
    #[tokio::test]
    async fn a_providers_records_are_projected_by_ref() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        let r = "work_item:fake:W-1";
        svc.event_log_store
            .append(recorded(r, "first", false))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row(svc, r).await,
            Some(("first".into(), "in_progress".into(), false))
        );
        svc.event_log_store
            .append(recorded(r, "renamed", true))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row(svc, r).await,
            Some(("renamed".into(), "in_progress".into(), true))
        );

        let own = oxplow_domain::refs::build::work_item_ref(fx.task);
        svc.event_log_store
            .append(recorded(&own, "hijack", false))
            .await
            .unwrap();
        let report = svc.event_pump.run_once().await.unwrap();
        assert_eq!(report.dead_lettered, 1);
        assert_ne!(row(svc, &own).await.unwrap().0, "hijack");
        assert!(WorkItemsProjection.handles(WorkItemRecorded::TYPE));
    }

    /// tsk1041: an outside tracker's item keeps the thread that filed it
    /// (the record's thread anchor, at its first record), so "This thread"
    /// lists it; a later restatement from elsewhere doesn't move it.
    #[tokio::test]
    async fn an_outside_item_keeps_the_thread_that_filed_it() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        let r = "work_item:fake:W-7";
        let anchored = |title: &str| {
            recorded(r, title, false).with_anchors(oxplow_domain::Anchors {
                thread_id: Some(fx.thread),
                ..Default::default()
            })
        };
        svc.event_log_store.append(anchored("Kiwi")).await.unwrap();
        svc.event_log_store
            .append(recorded(r, "Kiwi, renamed by a sync", false))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let out = svc
            .sql
            .query_sql(
                "SELECT thread_id FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(r.into())],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.rows).unwrap(),
            json!([[fx.thread.value()]])
        );
        // An oxplow task's thread is the task's own.
        let own = oxplow_domain::refs::build::work_item_ref(fx.task);
        let out = svc
            .sql
            .query_sql(
                "SELECT thread_id FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(own)],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.rows).unwrap(),
            json!([[fx.thread.value()]])
        );
    }
}
