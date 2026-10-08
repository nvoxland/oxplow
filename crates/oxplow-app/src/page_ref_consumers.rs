//! Event consumers that keep the `page_ref` graph current
//! (`.context/data-model.md` "page_ref", "event_log").
//!
//! A work item's page refs — its body's mentions, its links and its
//! comments' mentions — are restated from the work-item interface on any
//! of its events, in the pump's transaction: checkpointed, retried and
//! dead-lettered like any other consumer, so a crash between a write and
//! the projection can't leave the graph stale. The events are core's, the
//! same for every list (`.context/work-items.md`); a list's
//! `work_item.recorded` (a write's answer, a read back) restates it too.

use oxplow_domain::events::schema::EventType;
use oxplow_domain::events::schema::{
    WorkItemCommented, WorkItemCreated, WorkItemDeleted, WorkItemEdited, WorkItemLinked,
    WorkItemRecorded,
};
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::EventConsumer;

/// Restates a work item's `page_ref` slices from the interface
/// ([`oxplow_db::work_item_refs::restate_tx`]) on its events, whichever
/// list it's on. It runs after `work_items.project` (registered before
/// it), so a list's record is in the interface when it reads.
pub struct PageRefWorkItemConsumer {
    /// The ref kinds a body's mentions may name.
    pub vocabulary: VocabularyHandle,
}

impl PageRefWorkItemConsumer {
    pub const NAME: &'static str = "page_ref.work_item";
}

/// The events that name an item whose page refs may have changed.
const HANDLED: [&str; 6] = [
    WorkItemCreated::TYPE,
    WorkItemEdited::TYPE,
    WorkItemLinked::TYPE,
    WorkItemCommented::TYPE,
    WorkItemDeleted::TYPE,
    WorkItemRecorded::TYPE,
];

impl EventConsumer for PageRefWorkItemConsumer {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        HANDLED.contains(&event_type)
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let payload = &event.envelope.payload;
        let subject = payload
            .get("work_item")
            .or_else(|| payload.get("item").and_then(|item| item.get("ref")))
            .and_then(|v| v.as_str())
            .ok_or_else(|| DomainError::Invalid("payload names no work item".into()))?;
        let vocabulary = self.vocabulary.current();
        let r = oxplow_domain::refs::validate_ref(&vocabulary.kinds, subject)?;
        if r.kind != "work_item" {
            return Err(DomainError::Invalid(format!(
                "subject `{subject}` is not a work_item"
            )));
        }
        oxplow_db::work_item_refs::restate_tx(conn, &vocabulary.kinds, subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::events::schema::WorkItemEditedV2;
    use oxplow_domain::Envelope;

    fn edited(item: &str) -> StoredEvent {
        StoredEvent {
            seq: 1,
            envelope: Envelope::typed::<WorkItemEdited>(
                "test",
                &WorkItemEditedV2 {
                    work_item: item.into(),
                    fields: vec!["body".into()],
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
    /// list's `work_item.recorded` included — and gone with it.
    #[tokio::test]
    async fn an_items_links_and_comments_are_its_edges_from_the_interface() {
        use oxplow_domain::events::schema::{
            WorkItemCommented, WorkItemCommentedV2, WorkItemDeleted, WorkItemDeletedV2,
            WorkItemLinked, WorkItemLinkedV2,
        };
        let db = Database::in_memory();
        let consumer = PageRefWorkItemConsumer {
            vocabulary: VocabularyHandle::core(),
        };
        let item = "work_item:issues:ENG-1";
        let edges = |conn: &rusqlite::Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT ref_type, target_kind, target_id FROM page_ref
                     WHERE source_kind = 'work_item' AND source_id = 'issues:ENG-1'
                     ORDER BY ref_type, target_id",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok(format!(
                    "{} {}:{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        db.transaction(move |conn| {
            let run = |sql: &str| conn.execute(sql, []).map(|_| ()).map_err(oxplow_db::map_sql_err);
            run("INSERT INTO work_item (ref, provider, title, body, state, native_state, created_at, updated_at)
                 VALUES ('work_item:issues:ENG-1', 'issues', 'Fix it', 'see [[src/app.rs]]', 'todo', 'Todo', 't', 't')")?;
            // A list's own link type.
            run("INSERT INTO work_item_link (from_ref, to_ref, link_type, created_at)
                 VALUES ('work_item:issues:ENG-1', 'work_item:issues:ENG-2', 'parent_of', 't')")?;
            consumer.handle(
                conn,
                &event::<WorkItemLinked>(&WorkItemLinkedV2 {
                    work_item: item.into(),
                    target: "work_item:issues:ENG-2".into(),
                    link_type: "parent_of".into(),
                }),
            )?;
            run("INSERT INTO work_item_comment (id, ref, body, created_at)
                 VALUES ('work_item:issues:ENG-1#c1', 'work_item:issues:ENG-1', 'and [[src/b.rs]]', 't')")?;
            consumer.handle(
                conn,
                &event::<WorkItemCommented>(&WorkItemCommentedV2 {
                    work_item: item.into(),
                    comment: Some("c1".into()),
                }),
            )?;
            assert_eq!(
                edges(conn),
                vec![
                    "comment_file_ref file:src/b.rs",
                    "wiki_file_ref file:src/app.rs",
                    "work_item_link:parent_of work_item:issues:ENG-2",
                ]
            );
            // A read back restating the item without its link.
            run("DELETE FROM work_item_link")?;
            let recorded = oxplow_domain::events::schema::WorkItemRecordedV2 {
                item: serde_json::from_value(serde_json::json!({
                    "ref": item, "title": "Fix it", "state": "todo", "native_state": "Todo",
                    "links": []
                }))
                .unwrap(),
            };
            consumer.handle(
                conn,
                &event::<oxplow_domain::events::schema::WorkItemRecorded>(&recorded),
            )?;
            assert_eq!(
                edges(conn),
                vec![
                    "comment_file_ref file:src/b.rs",
                    "wiki_file_ref file:src/app.rs",
                ]
            );
            run("UPDATE work_item SET deleted_at = 't'")?;
            consumer.handle(
                conn,
                &event::<WorkItemDeleted>(&WorkItemDeletedV2 {
                    work_item: item.into(),
                }),
            )?;
            assert_eq!(edges(conn), Vec::<String>::new());
            Ok(())
        })
        .await
        .unwrap();
    }
}
