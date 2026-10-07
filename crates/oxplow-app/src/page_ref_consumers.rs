//! Event consumers that keep the `page_ref` graph current
//! (`.context/data-model.md` "page_ref", "event_log").
//!
//! When a task's own fields are edited (`work_item.edited`), re-project
//! its body-mention edges in the pump's transaction — checkpointed,
//! retried and dead-lettered like any other consumer, so a crash between
//! the edit and the projection can't leave the graph stale. (It used to
//! follow `work_item.transitioned`, whose status change moves no body
//! edge.)

use oxplow_db::page_ref_projections::{task_body_ref_types, work_item_edges, KIND_WORK_ITEM};
use oxplow_db::page_ref_store::replace_source_for_ref_types_tx;
use oxplow_domain::events::schema::EventType;
use oxplow_domain::events::schema::{WorkItemCreated, WorkItemEdited};
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{DomainError, StoredEvent};
use rusqlite::OptionalExtension as _;

use crate::event_pump::EventConsumer;

/// Projects a work item's body-mention `page_ref` edges on
/// `work_item.created` and re-projects them on `work_item.edited`, from the
/// item as the interface holds it (`work_item`), whichever list it's on.
pub struct PageRefWorkItemConsumer {
    /// The ref kinds a body's mentions may name.
    pub vocabulary: VocabularyHandle,
}

impl PageRefWorkItemConsumer {
    pub const NAME: &'static str = "page_ref.work_item";
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
        let r = oxplow_domain::refs::validate_ref(&vocabulary.kinds, subject)?;
        if r.kind != "work_item" {
            return Err(DomainError::Invalid(format!(
                "subject `{subject}` is not a work_item"
            )));
        }
        // The item as the interface holds it, whichever list it's on; one
        // deleted after the event was logged has nothing to project (its
        // rows go with the delete).
        let Some((title, body)) = conn
            .query_row(
                "SELECT title, body FROM work_item WHERE ref = ?1",
                [subject],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)?
        else {
            return Ok(());
        };
        replace_source_for_ref_types_tx(
            conn,
            KIND_WORK_ITEM,
            &r.id,
            &task_body_ref_types(),
            work_item_edges(&vocabulary.kinds, &r.id, &title, &body),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::events::schema::WorkItemEditedV1;
    use oxplow_domain::Envelope;

    fn edited(item: &str) -> StoredEvent {
        StoredEvent {
            seq: 1,
            envelope: Envelope::typed::<WorkItemEdited>(
                "test",
                &WorkItemEditedV1 {
                    work_item: item.into(),
                    fields: vec!["description".into()],
                },
            ),
            payload_expired_at: None,
        }
    }

    /// Any list's item gets its body-mention edges, read from the
    /// interface — not only oxplow's tasks.
    #[tokio::test]
    async fn any_lists_item_projects_its_mentions() {
        let db = Database::in_memory();
        let consumer = PageRefWorkItemConsumer {
            vocabulary: VocabularyHandle::core(),
        };
        let edges = db
            .transaction(move |conn| {
                conn.execute(
                    "INSERT INTO work_item (ref, provider, title, body, state, native_state, created_at, updated_at)
                     VALUES ('work_item:issues:ENG-1', 'issues', 'Fix it', 'see [[src/app.rs]]', 'todo', 'Todo', 't', 't')",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                consumer.handle(conn, &edited("work_item:issues:ENG-1"))?;
                // A deleted (unknown) item has nothing to project.
                consumer.handle(conn, &edited("work_item:issues:GONE"))?;
                assert!(consumer.handle(conn, &edited("commit:abc")).is_err());
                conn.query_row(
                    "SELECT source_id, target_kind, target_id FROM page_ref WHERE source_kind = 'work_item'",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(
            edges,
            ("issues:ENG-1".into(), "file".into(), "src/app.rs".into())
        );
    }
}
