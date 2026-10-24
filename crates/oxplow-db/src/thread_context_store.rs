//! The thread's context basket (`thread_context_item`): pages and pieces
//! of text the person collected for the thread's agents. The next prompt
//! to any agent in the thread carries every undelivered item and stamps it
//! delivered (`oxplow_app::agent_context`). Read through
//! `v_thread_context`; every write is an `oxplow.context.*` command over
//! these `_tx` cores. See migration `V107__thread_context.sql`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use oxplow_domain::{DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// What an item is: a page, by its canonical URI, or a piece of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Page,
    Text,
}

impl ContextKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Text => "text",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "text" {
            Self::Text
        } else {
            Self::Page
        }
    }
}

/// One item of a thread's basket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: i64,
    pub thread_id: i64,
    pub kind: ContextKind,
    /// The page (a `page` item), or where the text came from.
    pub source_uri: Option<String>,
    pub label: Option<String>,
    pub text: Option<String>,
    pub added_at: String,
    pub delivered_at: Option<String>,
}

/// What a new item holds, before it has an id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewItem {
    pub source_uri: Option<String>,
    pub label: Option<String>,
    pub text: Option<String>,
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextItem> {
    let kind: String = row.get("kind")?;
    Ok(ContextItem {
        id: row.get("id")?,
        thread_id: row.get("thread_id")?,
        kind: ContextKind::parse(&kind),
        source_uri: row.get("source_uri")?,
        label: row.get("label")?,
        text: row.get("text")?,
        added_at: row.get("added_at")?,
        delivered_at: row.get("delivered_at")?,
    })
}

/// Add an item to `thread`'s basket: a page when it has only a `source_uri`, a
/// piece of text when it has `text`. Its id.
pub fn add_tx(conn: &Connection, thread: ThreadId, item: &NewItem) -> Result<i64, DomainError> {
    let kind = match (&item.source_uri, &item.text) {
        (_, Some(t)) if !t.trim().is_empty() => ContextKind::Text,
        (Some(u), None) if !u.trim().is_empty() => ContextKind::Page,
        _ => {
            return Err(DomainError::Invalid(
                "a context item is a page (`source_uri`) or a piece of text (`text`)".into(),
            ))
        }
    };
    conn.execute(
        "INSERT INTO thread_context_item (thread_id, kind, source_uri, label, text, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            thread.value(),
            kind.as_str(),
            item.source_uri,
            item.label,
            item.text,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// The item `id`, if it exists.
pub fn get_tx(conn: &Connection, id: i64) -> Result<Option<ContextItem>, DomainError> {
    conn.query_row(
        "SELECT * FROM thread_context_item WHERE id = ?1",
        params![id],
        row_to_item,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Take item `id` out of its basket; what it was, if it was there.
pub fn remove_tx(conn: &Connection, id: i64) -> Result<Option<ContextItem>, DomainError> {
    let before = get_tx(conn, id)?;
    conn.execute("DELETE FROM thread_context_item WHERE id = ?1", params![id])
        .map_err(map_sql_err)?;
    Ok(before)
}

/// `thread`'s undelivered items, oldest first.
pub fn undelivered_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Vec<ContextItem>, DomainError> {
    conn.prepare(
        "SELECT * FROM thread_context_item
          WHERE thread_id = ?1 AND delivered_at IS NULL
          ORDER BY added_at, id",
    )
    .and_then(|mut stmt| {
        stmt.query_map(params![thread.value()], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .map_err(map_sql_err)
}

/// Empty `thread`'s basket of what is still waiting; what it held.
pub fn clear_tx(conn: &Connection, thread: ThreadId) -> Result<Vec<ContextItem>, DomainError> {
    let before = undelivered_tx(conn, thread)?;
    conn.execute(
        "DELETE FROM thread_context_item WHERE thread_id = ?1 AND delivered_at IS NULL",
        params![thread.value()],
    )
    .map_err(map_sql_err)?;
    Ok(before)
}

/// Stamp items `ids` delivered to agent session `session` (those still
/// waiting): each reaches the thread's agents once.
pub fn mark_delivered_tx(
    conn: &Connection,
    ids: &[i64],
    session: Option<i64>,
) -> Result<(), DomainError> {
    let now = ts_to_string(Timestamp::now());
    for id in ids {
        conn.execute(
            "UPDATE thread_context_item SET delivered_at = ?2, delivered_session_id = ?3
              WHERE id = ?1 AND delivered_at IS NULL",
            params![id, now, session],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    async fn with_thread() -> (Database, ThreadId) {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                   VALUES (1, 1, 'T', '2026-01-01', '2026-01-01');",
            )
            .map_err(map_sql_err)
        })
        .await
        .unwrap();
        let thread = ThreadId::new(1);
        (db, thread)
    }

    /// A page and a piece of text go in; the text wins the kind when an
    /// item has both (its `source_uri` is where it came from); nothing is refused.
    #[tokio::test]
    async fn items_are_pages_or_text_and_wait_in_order() {
        let (db, thread) = with_thread().await;
        let (page, text) = db
            .transaction(move |tx| {
                let page = add_tx(
                    tx,
                    thread,
                    &NewItem {
                        source_uri: Some("oxplow:oxplow/commit?sha=abc".into()),
                        label: Some("abc".into()),
                        text: None,
                    },
                )?;
                let text = add_tx(
                    tx,
                    thread,
                    &NewItem {
                        source_uri: Some("oxplow:oxplow/file?path=a.rs#lines=1-2".into()),
                        label: None,
                        text: Some("fn a() {}".into()),
                    },
                )?;
                Ok((page, text))
            })
            .await
            .unwrap();
        let waiting = db.read(move |c| undelivered_tx(c, thread)).await.unwrap();
        assert_eq!(
            waiting.iter().map(|i| (i.id, i.kind)).collect::<Vec<_>>(),
            [(page, ContextKind::Page), (text, ContextKind::Text)]
        );
        let empty = db
            .transaction(move |tx| add_tx(tx, thread, &NewItem::default()))
            .await;
        assert!(empty.is_err());
    }

    /// Delivered items leave the basket; clear and remove take only what
    /// they're asked for and say what it was.
    #[tokio::test]
    async fn delivery_clear_and_remove() {
        let (db, thread) = with_thread().await;
        let ids = db
            .transaction(move |tx| {
                let mut ids = Vec::new();
                for n in 0..3 {
                    ids.push(add_tx(
                        tx,
                        thread,
                        &NewItem {
                            source_uri: Some(format!("oxplow:oxplow/wiki?slug={n}")),
                            ..NewItem::default()
                        },
                    )?);
                }
                Ok(ids)
            })
            .await
            .unwrap();
        let first = ids[0];
        db.transaction(move |tx| mark_delivered_tx(tx, &[first], Some(7)))
            .await
            .unwrap();
        let waiting = db.read(move |c| undelivered_tx(c, thread)).await.unwrap();
        assert_eq!(waiting.len(), 2);
        let second = ids[1];
        let removed = db
            .transaction(move |tx| remove_tx(tx, second))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            removed.source_uri.as_deref(),
            Some("oxplow:oxplow/wiki?slug=1")
        );
        let cleared = db
            .transaction(move |tx| clear_tx(tx, thread))
            .await
            .unwrap();
        assert_eq!(cleared.len(), 1);
        assert!(db
            .read(move |c| undelivered_tx(c, thread))
            .await
            .unwrap()
            .is_empty());
        // The delivered one stays, as a record of what went out.
        assert!(db
            .read(move |c| get_tx(c, first))
            .await
            .unwrap()
            .unwrap()
            .delivered_at
            .is_some());
    }
}
