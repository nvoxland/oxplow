//! The work-items capability in the app (`.context/work-items.md`):
//!
//! - [`OxplowWorkItems`], the built-in provider — oxplow's own tasks —
//!   which writes through the `work_item.*` bus commands, the one write
//!   path for tasks;
//! - [`WorkItemsProjection`], the pump consumer (`work_items.project`)
//!   that upserts another provider's items into `work_item` from its
//!   `work_item.recorded` events. oxplow's own rows never take this path:
//!   the task cores write them with the task.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::events::schema::{EventType, WorkItemRecorded, WorkItemRecordedV1};
use oxplow_domain::work_items::{
    provider_of, CanonicalState, NewWorkItem, Transition, WorkItemPatch, WorkItemsError,
    WorkItemsFeatures, WorkItemsProvider,
};
use oxplow_domain::{Actor, CommandError, DomainError, StoredEvent, TaskStatus};
use serde_json::{json, Value};

use crate::commands::{work_item, CommandBus};
use crate::event_pump::EventConsumer;

/// oxplow's own provider name: `work_item:oxplow:tsk<n>`.
pub const PROVIDER: &str = "oxplow";

/// oxplow's status for a canonical state.
pub fn native_status(state: CanonicalState) -> TaskStatus {
    match state {
        CanonicalState::Todo => TaskStatus::Ready,
        CanonicalState::InProgress => TaskStatus::InProgress,
        CanonicalState::Blocked => TaskStatus::Blocked,
        CanonicalState::Done => TaskStatus::Done,
        CanonicalState::Canceled => TaskStatus::Canceled,
    }
}

/// oxplow's tasks as a [`WorkItemsProvider`]: each call is a `work_item.*`
/// command run as the given actor, so it is audited and policy-checked
/// like any other write. Holds the bus weakly — the bus's commands hold
/// the registry this provider sits in.
pub struct OxplowWorkItems {
    bus: Weak<CommandBus>,
}

impl OxplowWorkItems {
    pub fn new(bus: &Arc<CommandBus>) -> Self {
        Self {
            bus: Arc::downgrade(bus),
        }
    }

    async fn run(&self, actor: &Actor, name: &str, input: Value) -> Result<Value, WorkItemsError> {
        let bus = self
            .bus
            .upgrade()
            .ok_or_else(|| WorkItemsError::Failed("the command bus is gone".into()))?;
        bus.run(actor, name, input, false)
            .await
            .map(|outcome| outcome.result)
            .map_err(|e: CommandError| WorkItemsError::Failed(e.to_string()))
    }
}

#[async_trait]
impl WorkItemsProvider for OxplowWorkItems {
    fn provider(&self) -> &str {
        PROVIDER
    }

    fn features(&self) -> WorkItemsFeatures {
        WorkItemsFeatures {
            hierarchy: true,
            comments: true,
            links: true,
            in_progress_opens_effort: true,
        }
    }

    async fn create(&self, actor: &Actor, item: NewWorkItem) -> Result<String, WorkItemsError> {
        let result = self
            .run(
                actor,
                work_item::CREATE,
                // Filed on the actor's thread (the backlog for a person).
                json!({
                    "title": item.title,
                    "description": item.body,
                    "parent_ref": item.parent_ref,
                    "thread": actor.thread_id().map(|t| t.to_string()),
                }),
            )
            .await?;
        result["ref"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| WorkItemsError::Failed("work_item.create returned no ref".into()))
    }

    async fn update(
        &self,
        actor: &Actor,
        item_ref: &str,
        patch: WorkItemPatch,
    ) -> Result<(), WorkItemsError> {
        let mut input = json!({ "ref": item_ref });
        if let Some(title) = patch.title {
            input["title"] = title.into();
        }
        if let Some(body) = patch.body {
            input["description"] = body.into();
        }
        if let Some(parent) = patch.parent_ref {
            input["parent_ref"] = parent.unwrap_or_default().into();
        }
        self.run(actor, work_item::UPDATE, input).await.map(|_| ())
    }

    async fn transition(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: Transition,
    ) -> Result<(), WorkItemsError> {
        let to = match to {
            Transition::Canonical(state) => native_status(state),
            Transition::Native(raw) => {
                serde_json::from_value(Value::String(raw.clone())).map_err(|_| {
                    WorkItemsError::Failed(format!(
                        "`{raw}` isn't an oxplow status (ready, in_progress, blocked, done, \
                         canceled, archived)"
                    ))
                })?
            }
        };
        self.run(actor, work_item::NAME, json!({ "ref": item_ref, "to": to }))
            .await
            .map(|_| ())
    }

    async fn link(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<(), WorkItemsError> {
        self.run(
            actor,
            work_item::LINK,
            json!({ "ref": from, "target": to, "link_type": link_type }),
        )
        .await
        .map(|_| ())
    }

    async fn comment(
        &self,
        actor: &Actor,
        item_ref: &str,
        body: &str,
    ) -> Result<(), WorkItemsError> {
        self.run(
            actor,
            work_item::COMMENT,
            json!({ "ref": item_ref, "body": body }),
        )
        .await
        .map(|_| ())
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
                                    parent_ref, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, CASE WHEN ?10 THEN ?9 END)
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
            ],
        )
        .map_err(|e| DomainError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::EventType;
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
        let fx = crate::test_fixtures::services_with_effort().await;
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
}
