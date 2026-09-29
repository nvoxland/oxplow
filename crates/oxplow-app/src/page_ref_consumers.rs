//! Event consumers that keep the `page_ref` graph current
//! (`.context/data-model.md` "page_ref", "event_log").
//!
//! The first consumer on the pump: when a task transitions, re-project
//! its body-mention edges. Before the event log, the store did this
//! post-commit inside `update_with_effort_transition`; now the log is the
//! trigger, so the projection is checkpointed, retried and dead-lettered
//! like any other consumer, and a crash between the commit and the
//! projection can no longer leave the graph stale.

use oxplow_db::page_ref_projections::{
    task_body_ref_types, task_edges, work_item_id, KIND_WORK_ITEM,
};
use oxplow_db::page_ref_store::replace_source_for_ref_types_tx;
use oxplow_db::task_store::get_task_tx;
use oxplow_domain::events::schema::EventType;
use oxplow_domain::events::schema::WorkItemTransitioned;
use oxplow_domain::refs::grammar::CanonicalRef;
use oxplow_domain::{DomainError, StoredEvent, TaskId};

use crate::event_pump::EventConsumer;

/// Re-projects a task's body-mention `page_ref` edges on
/// `work_item.transitioned`.
pub struct PageRefWorkItemConsumer;

impl PageRefWorkItemConsumer {
    pub const NAME: &'static str = "page_ref.work_item";
}

/// The task behind a `work_item:oxplow:tskN` ref; `None` for another
/// provider's item, which has no page here.
fn task_of(subject: &str) -> Result<Option<TaskId>, DomainError> {
    let r = CanonicalRef::parse(subject)
        .map_err(|e| DomainError::Invalid(format!("subject `{subject}`: {}", e.reason())))?;
    if r.kind != "work_item" {
        return Err(DomainError::Invalid(format!(
            "subject `{subject}` is not a work_item"
        )));
    }
    let Some(bare) = r.id.strip_prefix("oxplow:") else {
        return Ok(None);
    };
    bare.parse::<TaskId>()
        .map(Some)
        .map_err(|e| DomainError::Invalid(format!("subject `{subject}`: {e}")))
}

impl EventConsumer for PageRefWorkItemConsumer {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemTransitioned::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let subject = event
            .envelope
            .payload
            .get("work_item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DomainError::Invalid("payload has no `work_item`".into()))?;
        let Some(task_id) = task_of(subject)? else {
            return Ok(());
        };
        // A task deleted after the event was logged has nothing to
        // project; its rows go with the delete.
        let Some(task) = get_task_tx(conn, task_id)? else {
            return Ok(());
        };
        replace_source_for_ref_types_tx(
            conn,
            KIND_WORK_ITEM,
            &work_item_id(task.id),
            &task_body_ref_types(),
            task_edges(&task),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_parsing_names_oxplow_tasks_only() {
        assert_eq!(
            task_of("work_item:oxplow:tsk42").unwrap(),
            Some(TaskId::new(42))
        );
        assert_eq!(task_of("work_item:linear:ENG-12").unwrap(), None);
        assert!(task_of("commit:abc").is_err());
        assert!(task_of("work_item:oxplow:nope").is_err());
        assert!(task_of("not a ref").is_err());
    }
}
