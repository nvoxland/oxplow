//! A task as the task interface records it: what every verb answers
//! with (`task.recorded`), read from the task tables in the verb's
//! transaction — the item whole, its links and comments included,
//! so the interface restates it from the record alone.

use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::ids::OxtaskId;
use oxplow_db::map_sql_err;
use oxplow_domain::tasks::{CommentRecord, LinkRecord, TaskRecord};
use oxplow_domain::{DomainError, NoteId};

use crate::mapping::status_str;
use crate::model::OxtaskStatus;
use crate::refs::task_ref;

/// Oxtask `id` as recorded — a deleted one too (`deleted`); `NotFound` for
/// none.
pub fn record_tx(conn: &rusqlite::Connection, id: OxtaskId) -> Result<TaskRecord, DomainError> {
    let row = conn
        .query_row(
            "SELECT title, description, status, priority, author,
                    parent_id, deleted_at IS NOT NULL
               FROM oxtask WHERE id = ?1",
            params![id.value()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?
        .ok_or(DomainError::NotFound)?;
    let (title, body, status, priority, author, parent, deleted) = row;
    let status: OxtaskStatus = serde_json::from_value(json!(status))
        .map_err(|e| DomainError::Invalid(format!("task {id}'s status: {e}")))?;
    let links = rows(
        conn,
        "SELECT to_uri, link_type FROM oxtask_link WHERE from_item_id = ?1
         ORDER BY created_at, id",
        id,
        |r| {
            Ok(LinkRecord {
                target: r.get(0)?,
                link_type: r.get(1)?,
            })
        },
    )?;
    let comments = rows(
        conn,
        "SELECT id, body, author, created_at FROM oxtask_note WHERE task_id = ?1
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
    Ok(TaskRecord {
        uri: task_ref(id),
        title,
        body,
        stage: status_str(status),
        native: json!({ "priority": priority, "author": author }),
        parent_uri: parent.map(|p| task_ref(OxtaskId::new(p))),
        deleted,
        links: Some(links),
        comments: Some(comments),
    })
}

fn rows<T>(
    conn: &rusqlite::Connection,
    sql: &str,
    id: OxtaskId,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, DomainError> {
    let mut stmt = conn.prepare(sql).map_err(map_sql_err)?;
    let out = stmt
        .query_map(params![id.value()], map)
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<T>>>())
        .map_err(map_sql_err);
    out
}
