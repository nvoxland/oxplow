//! Event consumers that keep the `page_ref` graph current
//! (`.context/data-model.md` "page_ref", "event_log").
//!
//! When a task's own fields are edited (`work_item.edited`), re-project
//! its body-mention edges in the pump's transaction — checkpointed,
//! retried and dead-lettered like any other consumer, so a crash between
//! the edit and the projection can't leave the graph stale. (It used to
//! follow `work_item.transitioned`, whose status change moves no body
//! edge.)

use oxplow_db::page_ref_projections::{
    task_body_ref_types, task_edges, work_item_id, KIND_WORK_ITEM,
};
use oxplow_db::page_ref_store::replace_source_for_ref_types_tx;
use oxplow_db::task_store::get_task_tx;
use oxplow_domain::events::schema::EventType;
use oxplow_domain::events::schema::{WorkItemCreated, WorkItemEdited};
use oxplow_domain::refs::kind::KindRegistry;
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{DomainError, StoredEvent, TaskId};

use crate::event_pump::EventConsumer;

/// Projects a task's body-mention `page_ref` edges on `work_item.created`
/// and re-projects them on `work_item.edited`.
pub struct PageRefWorkItemConsumer {
    /// The ref kinds a body's mentions may name.
    pub vocabulary: VocabularyHandle,
}

impl PageRefWorkItemConsumer {
    pub const NAME: &'static str = "page_ref.work_item";
}

/// The task behind a `work_item:oxplow:tskN` ref; `None` for another
/// provider's item, which has no page here.
fn task_of(kinds: &KindRegistry, subject: &str) -> Result<Option<TaskId>, DomainError> {
    let r = oxplow_domain::refs::validate_ref(kinds, subject)?;
    if r.kind != "work_item" {
        return Err(DomainError::Invalid(format!(
            "subject `{subject}` is not a work_item"
        )));
    }
    if !r.id.starts_with("oxplow:") {
        return Ok(None);
    }
    oxplow_domain::refs::build::task_of_work_item_ref(subject)
        .map(Some)
        .ok_or_else(|| DomainError::Invalid(format!("`{subject}` names no oxplow task")))
}

impl EventConsumer for PageRefWorkItemConsumer {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemEdited::TYPE || event_type == WorkItemCreated::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let subject = event
            .envelope
            .payload
            .get("work_item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DomainError::Invalid("payload has no `work_item`".into()))?;
        let vocabulary = self.vocabulary.current();
        let Some(task_id) = task_of(&vocabulary.kinds, subject)? else {
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
            task_edges(&vocabulary.kinds, &task),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::refs::kind::core_kinds;

    #[test]
    fn subject_parsing_names_oxplow_tasks_only() {
        assert_eq!(
            task_of(&core_kinds(), "work_item:oxplow:tsk42").unwrap(),
            Some(TaskId::new(42))
        );
        assert_eq!(
            task_of(&core_kinds(), "work_item:linear:ENG-12").unwrap(),
            None
        );
        assert!(task_of(&core_kinds(), "commit:abc").is_err());
        assert!(task_of(&core_kinds(), "work_item:oxplow:nope").is_err());
        assert!(task_of(&core_kinds(), "not a ref").is_err());
    }
}
