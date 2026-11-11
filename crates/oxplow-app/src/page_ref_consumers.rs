//! Event consumers that keep the `page_ref` graph current
//! (`.context/data-model.md` "page_ref", "event_log").
//!
//! A task's page refs — its body's mentions, its links and its
//! comments' mentions — are restated from the task interface on any
//! of its events, in the pump's transaction: checkpointed, retried and
//! dead-lettered like any other consumer, so a crash between a write and
//! the projection can't leave the graph stale. The events are core's, the
//! same for every list (`.context/tasks.md`); a list's
//! `task.recorded` (a write's answer, a read back) restates it too.

use oxplow_domain::events::schema::EventType;
use oxplow_domain::events::schema::{
    TaskCommented, TaskCreated, TaskDeleted, TaskEdited, TaskLinked, TaskRecorded,
};
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::EventConsumer;

/// Restates a task's `page_ref` slices from the interface
/// ([`oxplow_db::task_refs::restate_tx`]) on its events, whichever
/// list it's on. It runs after `tasks.project` (registered before
/// it), so a list's record is in the interface when it reads.
pub struct PageRefTaskConsumer {
    /// The ref kinds a body's mentions may name.
    pub vocabulary: VocabularyHandle,
}

impl PageRefTaskConsumer {
    pub const NAME: &'static str = "page_ref.task";
}

/// The events that name an item whose page refs may have changed.
const HANDLED: [&str; 6] = [
    TaskCreated::TYPE,
    TaskEdited::TYPE,
    TaskLinked::TYPE,
    TaskCommented::TYPE,
    TaskDeleted::TYPE,
    TaskRecorded::TYPE,
];

impl EventConsumer for PageRefTaskConsumer {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        HANDLED.contains(&event_type)
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let payload = &event.envelope.payload;
        let subject = payload
            .get("task")
            .or_else(|| payload.get("item").and_then(|item| item.get("uri")))
            .and_then(|v| v.as_str())
            .ok_or_else(|| DomainError::Invalid("payload names no task".into()))?;
        let vocabulary = self.vocabulary.current();
        let r = oxplow_domain::uri::validate_uri(&vocabulary.resources, subject)?;
        if r.resource().to_string() != oxplow_domain::uri::resources::TASK {
            return Err(DomainError::Invalid(format!(
                "subject `{subject}` is not a task"
            )));
        }
        oxplow_db::task_refs::restate_tx(conn, &vocabulary.resources, subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::events::schema::TaskEditedV2;
    use oxplow_domain::Envelope;

    fn edited(item: &str) -> StoredEvent {
        StoredEvent {
            seq: 1,
            envelope: Envelope::typed::<TaskEdited>(
                "test",
                &TaskEditedV2 {
                    task: item.into(),
                    fields: vec!["body".into()],
                },
            ),
            payload_expired_at: None,
        }
    }

    /// Any list's item gets its body-mention edges, read from the
    /// interface — not only oxtask.
    #[tokio::test]
    async fn any_lists_item_projects_its_mentions() {
        let db = Database::in_memory();
        let consumer = PageRefTaskConsumer {
            vocabulary: VocabularyHandle::core(),
        };
        let edges = db
            .transaction(move |conn| {
                conn.execute(
                    "INSERT INTO task (uri, provider, title, body, state, stage, created_at, updated_at)
                     VALUES ('oxplow:oxplow/task?id=ENG-1&provider=issues', 'issues', 'Fix it', 'see [[src/app.rs]]', 'todo', 'Todo', 't', 't')",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                consumer.handle(conn, &edited("oxplow:oxplow/task?id=ENG-1&provider=issues"))?;
                // A deleted (unknown) item has nothing to project.
                consumer.handle(conn, &edited("oxplow:oxplow/task?id=GONE&provider=issues"))?;
                assert!(consumer.handle(conn, &edited("oxplow:oxplow/commit?sha=abc")).is_err());
                conn.query_row(
                    "SELECT source_uri, target_resource, target_uri FROM page_ref WHERE source_resource = 'oxplow/task'",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(
            edges,
            (
                "oxplow:oxplow/task?id=ENG-1&provider=issues".into(),
                "oxplow/file".into(),
                "oxplow:oxplow/file?path=src/app.rs".into()
            )
        );
    }

    fn event<T: EventType>(payload: &T::Payload) -> StoredEvent {
        StoredEvent {
            seq: 1,
            envelope: Envelope::typed::<T>("test", payload),
            payload_expired_at: None,
        }
    }

    /// An item's page refs are its body's mentions, its links (by the
    /// list's own link types) and its comments' mentions, all keyed by the
    /// item and restated from the interface on any of its events — the
    /// list's `task.recorded` included — and gone with it.
    #[tokio::test]
    async fn an_items_links_and_comments_are_its_edges_from_the_interface() {
        use oxplow_domain::events::schema::{
            TaskCommented, TaskCommentedV2, TaskDeleted, TaskDeletedV2, TaskLinked, TaskLinkedV2,
        };
        let db = Database::in_memory();
        let consumer = PageRefTaskConsumer {
            vocabulary: VocabularyHandle::core(),
        };
        let item = "oxplow:oxplow/task?id=ENG-1&provider=issues";
        let edges = |conn: &rusqlite::Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT ref_type, target_uri FROM page_ref
                     WHERE source_uri = 'oxplow:oxplow/task?id=ENG-1&provider=issues'
                     ORDER BY ref_type, target_uri",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok(format!(
                    "{} {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        db.transaction(move |conn| {
            let run = |sql: &str| conn.execute(sql, []).map(|_| ()).map_err(oxplow_db::map_sql_err);
            run("INSERT INTO task (uri, provider, title, body, state, stage, created_at, updated_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-1&provider=issues', 'issues', 'Fix it', 'see [[src/app.rs]]', 'todo', 'Todo', 't', 't')")?;
            // A list's own link type.
            run("INSERT INTO task_link (from_uri, to_uri, link_type, created_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-1&provider=issues', 'oxplow:oxplow/task?id=ENG-2&provider=issues', 'parent_of', 't')")?;
            consumer.handle(
                conn,
                &event::<TaskLinked>(&TaskLinkedV2 {
                    task: item.into(),
                    target: "oxplow:oxplow/task?id=ENG-2&provider=issues".into(),
                    link_type: "parent_of".into(),
                }),
            )?;
            run("INSERT INTO task_comment (id, uri, body, created_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-1&provider=issues#part=c1', 'oxplow:oxplow/task?id=ENG-1&provider=issues', 'and [[src/b.rs]]', 't')")?;
            consumer.handle(
                conn,
                &event::<TaskCommented>(&TaskCommentedV2 {
                    task: item.into(),
                    comment: Some("c1".into()),
                }),
            )?;
            assert_eq!(
                edges(conn),
                vec![
                    "comment_file_ref oxplow:oxplow/file?path=src/b.rs",
                    "file_mention oxplow:oxplow/file?path=src/app.rs",
                    "task_link:parent_of oxplow:oxplow/task?id=ENG-2&provider=issues",
                ]
            );
            // A read back restating the item without its link.
            run("DELETE FROM task_link")?;
            let recorded = oxplow_domain::events::schema::TaskRecordedV3 {
                item: serde_json::from_value(serde_json::json!({
                    "uri": item, "title": "Fix it", "stage": "todo",
                    "links": []
                }))
                .unwrap(),
            };
            consumer.handle(
                conn,
                &event::<oxplow_domain::events::schema::TaskRecorded>(&recorded),
            )?;
            assert_eq!(
                edges(conn),
                vec![
                    "comment_file_ref oxplow:oxplow/file?path=src/b.rs",
                    "file_mention oxplow:oxplow/file?path=src/app.rs",
                ]
            );
            run("UPDATE task SET deleted_at = 't'")?;
            consumer.handle(
                conn,
                &event::<TaskDeleted>(&TaskDeletedV2 {
                    task: item.into(),
                }),
            )?;
            assert_eq!(edges(conn), Vec::<String>::new());
            Ok(())
        })
        .await
        .unwrap();
    }
}
