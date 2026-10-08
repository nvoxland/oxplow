//! A task as the work-item interface records it: what every verb answers
//! with (`work_item.recorded`), read from the task tables in the verb's
//! transaction — the item whole, its links, comments and list included,
//! so the interface restates it from the record alone.

use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::ids::TaskId;
use oxplow_db::map_sql_err;
use oxplow_domain::work_items::{CommentRecord, LinkRecord, List, WorkItemRecord};
use oxplow_domain::{DomainError, NoteId, ThreadId};

use crate::mapping::state_pair;
use crate::model::TaskStatus;
use crate::refs::work_item_ref;

/// Task `id` as recorded — a deleted one too (`deleted`); `NotFound` for
/// none.
pub fn record_tx(conn: &rusqlite::Connection, id: TaskId) -> Result<WorkItemRecord, DomainError> {
    let row = conn
        .query_row(
            "SELECT title, description, status, completed_at IS NOT NULL, priority, author,
                    parent_id, deleted_at IS NOT NULL, sort_index, thread_id
               FROM task WHERE id = ?1",
            params![id.value()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, bool>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, Option<i64>>(9)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?
        .ok_or(DomainError::NotFound)?;
    let (title, body, status, completed, priority, author, parent, deleted, sort_index, thread) =
        row;
    let status: TaskStatus = serde_json::from_value(json!(status))
        .map_err(|e| DomainError::Invalid(format!("task {id}'s status: {e}")))?;
    let (state, native_state) = state_pair(status, completed);
    let links = rows(
        conn,
        "SELECT to_item_id, link_type FROM task_link WHERE from_item_id = ?1
         ORDER BY created_at, id",
        id,
        |r| {
            Ok(LinkRecord {
                target: work_item_ref(TaskId::new(r.get(0)?)),
                link_type: r.get(1)?,
            })
        },
    )?;
    let comments = rows(
        conn,
        "SELECT id, body, author, created_at FROM task_note WHERE task_id = ?1
         ORDER BY created_at, id",
        id,
        |r| {
            Ok(CommentRecord {
                id: NoteId::new(r.get(0)?).to_string(),
                body: r.get(1)?,
                author: Some(r.get(2)?),
                created_at: Some(r.get(3)?),
            })
        },
    )?;
    Ok(WorkItemRecord {
        item_ref: work_item_ref(id),
        title,
        body,
        state,
        native_state,
        native: json!({ "priority": priority, "author": author }),
        parent_ref: parent.map(|p| work_item_ref(TaskId::new(p))),
        deleted,
        rank: Some(sort_index as f64),
        links: Some(links),
        comments: Some(comments),
        list: Some(thread.map_or(List::Backlog, |t| {
            List::Thread(ThreadId::new(t).to_string())
        })),
    })
}

fn rows<T>(
    conn: &rusqlite::Connection,
    sql: &str,
    id: TaskId,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, DomainError> {
    let mut stmt = conn.prepare(sql).map_err(map_sql_err)?;
    let out = stmt
        .query_map(params![id.value()], map)
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<T>>>())
        .map_err(map_sql_err);
    out
}
