//! Context baskets (`context_item`, `context_basket`; V47): pages and
//! pieces of text the person collected for agents. Each item is in one
//! basket: a thread's own (shown as "This Thread") or a named basket of the
//! project, collected into from any stream and attached to a thread's next
//! prompt. The next prompt to any agent in a thread carries its own basket
//! and every basket attached to it, then deletes what it carried and
//! detaches those baskets (`oxplow_app::agent_context`). Read through
//! `v_context_item` and `v_context_basket`; every write is an
//! `oxplow.context.*` command over these `_tx` cores.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use oxplow_domain::{DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// What the thread's own basket is called, so no named basket may be.
pub const THREAD_BASKET: &str = "This Thread";
/// The longest a basket's name may be.
pub const NAME_MAX: usize = 60;

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

/// The basket an item is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basket {
    /// The thread's own.
    Thread(ThreadId),
    /// A named basket (`context_basket.id`).
    Named(i64),
}

impl Basket {
    fn columns(self) -> (Option<i64>, Option<i64>) {
        match self {
            Self::Thread(t) => (Some(t.value()), None),
            Self::Named(b) => (None, Some(b)),
        }
    }
}

/// One collected item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: i64,
    /// The thread whose own basket it is in, or
    pub thread_id: Option<i64>,
    /// the named basket it is in.
    pub basket_id: Option<i64>,
    /// The stream it was collected in, when known.
    pub stream_id: Option<i64>,
    pub kind: ContextKind,
    /// The page (a `page` item), or where the text came from.
    pub source_uri: Option<String>,
    pub label: Option<String>,
    pub text: Option<String>,
    /// The lines around `text`, when where it came from has no lines to
    /// read back (a terminal).
    pub around: Option<String>,
    /// What the person wants done about it, in their words.
    pub note: Option<String>,
    pub added_at: String,
}

impl ContextItem {
    /// The basket it is in.
    pub fn basket(&self) -> Basket {
        match (self.thread_id, self.basket_id) {
            (Some(t), _) => Basket::Thread(ThreadId::new(t)),
            (None, Some(b)) => Basket::Named(b),
            // The table's CHECK keeps an item in exactly one.
            (None, None) => unreachable!("a context item is in a basket"),
        }
    }
}

/// What a new item holds, before it has an id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewItem {
    pub stream_id: Option<i64>,
    pub source_uri: Option<String>,
    pub label: Option<String>,
    pub text: Option<String>,
    pub around: Option<String>,
    pub note: Option<String>,
}

impl NewItem {
    /// What `item` holds, to put it back.
    pub fn of(item: &ContextItem) -> Self {
        Self {
            stream_id: item.stream_id,
            source_uri: item.source_uri.clone(),
            label: item.label.clone(),
            text: item.text.clone(),
            around: item.around.clone(),
            note: item.note.clone(),
        }
    }
}

/// A named basket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedBasket {
    pub id: i64,
    pub name: String,
    /// The thread whose next prompt carries it, if any.
    pub attached_thread_id: Option<i64>,
    pub created_at: String,
}

/// The stream an item was collected in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemStream {
    pub id: i64,
    pub title: String,
    pub branch: String,
}

/// An item a thread's next prompt would carry, with the named basket it is
/// in (none: the thread's own) and the stream it was collected in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingItem {
    pub item: ContextItem,
    pub basket: Option<String>,
    pub stream: Option<ItemStream>,
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextItem> {
    let kind: String = row.get("kind")?;
    Ok(ContextItem {
        id: row.get("id")?,
        thread_id: row.get("thread_id")?,
        basket_id: row.get("basket_id")?,
        stream_id: row.get("stream_id")?,
        kind: ContextKind::parse(&kind),
        source_uri: row.get("source_uri")?,
        label: row.get("label")?,
        text: row.get("text")?,
        around: row.get("around")?,
        note: row.get("note")?,
        added_at: row.get("added_at")?,
    })
}

fn row_to_basket(row: &rusqlite::Row<'_>) -> rusqlite::Result<NamedBasket> {
    Ok(NamedBasket {
        id: row.get("id")?,
        name: row.get("name")?,
        attached_thread_id: row.get("attached_thread_id")?,
        created_at: row.get("created_at")?,
    })
}

/// Add an item to `basket`: a page when it has only a `source_uri`, a piece
/// of text when it has `text`. Its id.
pub fn add_tx(conn: &Connection, basket: Basket, item: &NewItem) -> Result<i64, DomainError> {
    let kind = match (&item.source_uri, &item.text) {
        (_, Some(t)) if !t.trim().is_empty() => ContextKind::Text,
        (Some(u), None) if !u.trim().is_empty() => ContextKind::Page,
        _ => {
            return Err(DomainError::Invalid(
                "a context item is a page (`source_uri`) or a piece of text (`text`)".into(),
            ))
        }
    };
    let (thread_id, basket_id) = basket.columns();
    conn.execute(
        "INSERT INTO context_item (thread_id, basket_id, stream_id, kind, source_uri, label, text, around, note, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            thread_id,
            basket_id,
            item.stream_id,
            kind.as_str(),
            item.source_uri,
            item.label,
            item.text,
            item.around,
            item.note,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// The item `id`, if it exists.
pub fn get_tx(conn: &Connection, id: i64) -> Result<Option<ContextItem>, DomainError> {
    conn.query_row(
        "SELECT * FROM context_item WHERE id = ?1",
        params![id],
        row_to_item,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Take item `id` out of its basket; what it was, if it was there.
pub fn remove_tx(conn: &Connection, id: i64) -> Result<Option<ContextItem>, DomainError> {
    let before = get_tx(conn, id)?;
    conn.execute("DELETE FROM context_item WHERE id = ?1", params![id])
        .map_err(map_sql_err)?;
    Ok(before)
}

/// Set or clear (`None`) the note of item `id`; what the item was, `None`
/// when there is no such item.
pub fn set_note_tx(
    conn: &Connection,
    id: i64,
    note: Option<&str>,
) -> Result<Option<ContextItem>, DomainError> {
    let Some(before) = get_tx(conn, id)? else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE context_item SET note = ?2 WHERE id = ?1",
        params![id, note],
    )
    .map_err(map_sql_err)?;
    Ok(Some(before))
}

/// Move item `id` into `basket`; what it was, `None` when there is no such
/// item.
pub fn move_tx(
    conn: &Connection,
    id: i64,
    basket: Basket,
) -> Result<Option<ContextItem>, DomainError> {
    let Some(before) = get_tx(conn, id)? else {
        return Ok(None);
    };
    let (thread_id, basket_id) = basket.columns();
    conn.execute(
        "UPDATE context_item SET thread_id = ?2, basket_id = ?3 WHERE id = ?1",
        params![id, thread_id, basket_id],
    )
    .map_err(map_sql_err)?;
    Ok(Some(before))
}

/// `basket`'s items, oldest first.
pub fn items_tx(conn: &Connection, basket: Basket) -> Result<Vec<ContextItem>, DomainError> {
    let (sql, id) = match basket {
        Basket::Thread(t) => (
            "SELECT * FROM context_item WHERE thread_id = ?1 ORDER BY added_at, id",
            t.value(),
        ),
        Basket::Named(b) => (
            "SELECT * FROM context_item WHERE basket_id = ?1 ORDER BY added_at, id",
            b,
        ),
    };
    conn.prepare(sql)
        .and_then(|mut stmt| {
            stmt.query_map(params![id], row_to_item)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)
}

/// Empty `basket`; what it held.
pub fn clear_tx(conn: &Connection, basket: Basket) -> Result<Vec<ContextItem>, DomainError> {
    let before = items_tx(conn, basket)?;
    for item in &before {
        conn.execute("DELETE FROM context_item WHERE id = ?1", params![item.id])
            .map_err(map_sql_err)?;
    }
    Ok(before)
}

/// A basket's name as kept: trimmed, 1 to [`NAME_MAX`] characters, and not
/// the thread's own basket's.
pub fn basket_name(raw: &str) -> Result<String, DomainError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(DomainError::Invalid("a basket needs a name".into()));
    }
    if name.chars().count() > NAME_MAX {
        return Err(DomainError::Invalid(format!(
            "a basket's name is at most {NAME_MAX} characters"
        )));
    }
    if name.eq_ignore_ascii_case(THREAD_BASKET) {
        return Err(DomainError::Invalid(format!(
            "\"{THREAD_BASKET}\" is the thread's own basket: name it something else"
        )));
    }
    Ok(name.to_string())
}

/// The named basket called `name` (any case), if there is one.
pub fn basket_by_name_tx(
    conn: &Connection,
    name: &str,
) -> Result<Option<NamedBasket>, DomainError> {
    conn.query_row(
        "SELECT * FROM context_basket WHERE name = ?1",
        params![name.trim()],
        row_to_basket,
    )
    .optional()
    .map_err(map_sql_err)
}

/// The named basket `id`, if it exists.
pub fn get_basket_tx(conn: &Connection, id: i64) -> Result<Option<NamedBasket>, DomainError> {
    conn.query_row(
        "SELECT * FROM context_basket WHERE id = ?1",
        params![id],
        row_to_basket,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Every named basket, by name.
pub fn baskets_tx(conn: &Connection) -> Result<Vec<NamedBasket>, DomainError> {
    conn.prepare("SELECT * FROM context_basket ORDER BY name")
        .and_then(|mut stmt| {
            stmt.query_map([], row_to_basket)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(map_sql_err)
}

/// Make a named basket; refused when the name is taken or not a name.
pub fn create_basket_tx(conn: &Connection, name: &str) -> Result<NamedBasket, DomainError> {
    let name = basket_name(name)?;
    if basket_by_name_tx(conn, &name)?.is_some() {
        return Err(DomainError::Invalid(format!(
            "there is a basket named \"{name}\" already"
        )));
    }
    conn.execute(
        "INSERT INTO context_basket (name, created_at) VALUES (?1, ?2)",
        params![name, ts_to_string(Timestamp::now())],
    )
    .map_err(map_sql_err)?;
    let id = conn.last_insert_rowid();
    Ok(get_basket_tx(conn, id)?.expect("the basket just made"))
}

/// The named basket called `name`, made when there is none; and whether it
/// was made.
pub fn ensure_basket_tx(conn: &Connection, name: &str) -> Result<(NamedBasket, bool), DomainError> {
    let name = basket_name(name)?;
    match basket_by_name_tx(conn, &name)? {
        Some(b) => Ok((b, false)),
        None => Ok((create_basket_tx(conn, &name)?, true)),
    }
}

/// Rename basket `id`; what it was. Refused when the name is another
/// basket's or not a name.
pub fn rename_basket_tx(
    conn: &Connection,
    id: i64,
    name: &str,
) -> Result<Option<NamedBasket>, DomainError> {
    let name = basket_name(name)?;
    let Some(before) = get_basket_tx(conn, id)? else {
        return Ok(None);
    };
    if basket_by_name_tx(conn, &name)?.is_some_and(|b| b.id != id) {
        return Err(DomainError::Invalid(format!(
            "there is a basket named \"{name}\" already"
        )));
    }
    conn.execute(
        "UPDATE context_basket SET name = ?2 WHERE id = ?1",
        params![id, name],
    )
    .map_err(map_sql_err)?;
    Ok(Some(before))
}

/// Delete basket `id` and its items; what they were.
pub fn delete_basket_tx(
    conn: &Connection,
    id: i64,
) -> Result<Option<(NamedBasket, Vec<ContextItem>)>, DomainError> {
    let Some(basket) = get_basket_tx(conn, id)? else {
        return Ok(None);
    };
    let items = items_tx(conn, Basket::Named(id))?;
    conn.execute("DELETE FROM context_item WHERE basket_id = ?1", params![id])
        .map_err(map_sql_err)?;
    conn.execute("DELETE FROM context_basket WHERE id = ?1", params![id])
        .map_err(map_sql_err)?;
    Ok(Some((basket, items)))
}

/// Attach basket `id` to `thread`'s next prompt (`None`: detach it); the
/// thread it was attached to before. A basket is attached to one thread at
/// a time.
pub fn attach_tx(
    conn: &Connection,
    id: i64,
    thread: Option<ThreadId>,
) -> Result<Option<i64>, DomainError> {
    let before =
        get_basket_tx(conn, id)?.ok_or_else(|| DomainError::Invalid(format!("no basket {id}")))?;
    conn.execute(
        "UPDATE context_basket SET attached_thread_id = ?2 WHERE id = ?1",
        params![id, thread.map(|t| t.value())],
    )
    .map_err(map_sql_err)?;
    Ok(before.attached_thread_id)
}

/// What `thread`'s next prompt would carry: its own items, then each
/// attached basket's (by name), each oldest first.
pub fn waiting_for_thread_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Vec<WaitingItem>, DomainError> {
    conn.prepare(
        "SELECT c.*, b.name AS basket_name,
                s.id AS s_id, s.title AS s_title, s.branch AS s_branch
           FROM context_item c
           LEFT JOIN context_basket b ON b.id = c.basket_id
           LEFT JOIN streams s ON s.id = c.stream_id
          WHERE c.thread_id = ?1 OR b.attached_thread_id = ?1
          ORDER BY c.basket_id IS NOT NULL, b.name COLLATE NOCASE, c.added_at, c.id",
    )
    .and_then(|mut stmt| {
        stmt.query_map(params![thread.value()], |row| {
            let stream = match row.get::<_, Option<i64>>("s_id")? {
                Some(id) => Some(ItemStream {
                    id,
                    title: row.get("s_title")?,
                    branch: row.get("s_branch")?,
                }),
                None => None,
            };
            Ok(WaitingItem {
                item: row_to_item(row)?,
                basket: row.get("basket_name")?,
                stream,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
    })
    .map_err(map_sql_err)
}

/// A prompt to `thread` carried `items` and the handoffs `handoffs`: record
/// the turn they landed in — the thread's open turn (of `session` when
/// known), which the prompt opened or went into — on the handoffs, link
/// that turn to what it was given (`prompt_mention`: each item's page or
/// source, each handoff), then delete the items and detach the baskets
/// they came from. With no turn open nothing is linked; the items still go.
pub fn take_delivered_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<i64>,
    handoffs: &[oxplow_domain::HandoffId],
    items: &[i64],
) -> Result<(), DomainError> {
    use oxplow_domain::uri::build::{handoff_uri, turn_uri};
    let turn: Option<i64> = conn
        .query_row(
            "SELECT id FROM agent_turn
              WHERE thread_id = ?1 AND ended_at IS NULL
                AND (?2 IS NULL OR agent_session_id = ?2)
              ORDER BY started_at DESC, id DESC LIMIT 1",
            params![thread.value(), session],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    let taken: Vec<ContextItem> = items
        .iter()
        .map(|id| get_tx(conn, *id))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    if let Some(turn) = turn {
        let source = turn_uri(oxplow_domain::AgentTurnId::new(turn));
        let mention = |target: String| {
            crate::page_ref_store::upsert_edge_tx(
                conn,
                &crate::page_ref_store::PageRefEdge::new(
                    &source,
                    target,
                    crate::page_ref_projections::RT_PROMPT_MENTION,
                ),
            )
        };
        for id in handoffs {
            conn.execute(
                "UPDATE handoff SET delivered_turn_id = ?2 WHERE id = ?1 AND delivered_turn_id IS NULL",
                params![id.value(), turn],
            )
            .map_err(map_sql_err)?;
            mention(handoff_uri(*id))?;
        }
        for item in &taken {
            if let Some(uri) = &item.source_uri {
                mention(uri.clone())?;
            }
        }
    }
    for item in &taken {
        conn.execute("DELETE FROM context_item WHERE id = ?1", params![item.id])
            .map_err(map_sql_err)?;
        if let Some(basket) = item.basket_id {
            conn.execute(
                "UPDATE context_basket SET attached_thread_id = NULL
                  WHERE id = ?1 AND attached_thread_id = ?2",
                params![basket, thread.value()],
            )
            .map_err(map_sql_err)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// Two streams with a thread each: thread 1 in stream 1 (`main`), thread
    /// 2 in stream 2 (`feature`).
    async fn with_threads() -> (Database, ThreadId, ThreadId) {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01'),
                          (2, 'worktree', 'Feature', 'feature', 'refs/heads/feature', 'local', '/tmp/y', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                   VALUES (1, 1, 'T', '2026-01-01', '2026-01-01'),
                          (2, 2, 'U', '2026-01-01', '2026-01-01');",
            )
            .map_err(map_sql_err)
        })
        .await
        .unwrap();
        (db, ThreadId::new(1), ThreadId::new(2))
    }

    fn page(uri: &str) -> NewItem {
        NewItem {
            source_uri: Some(uri.into()),
            ..NewItem::default()
        }
    }

    /// A page and a piece of text go in; the text wins the kind when an
    /// item has both (its `source_uri` is where it came from); nothing is refused.
    #[tokio::test]
    async fn items_are_pages_or_text_and_wait_in_order() {
        let (db, thread, _) = with_threads().await;
        let (p, t) = db
            .transaction(move |tx| {
                let p = add_tx(
                    tx,
                    Basket::Thread(thread),
                    &page("oxplow:oxplow/commit?sha=abc"),
                )?;
                let t = add_tx(
                    tx,
                    Basket::Thread(thread),
                    &NewItem {
                        source_uri: Some("oxplow:oxplow/file?path=a.rs#lines=1-2".into()),
                        text: Some("fn a() {}".into()),
                        ..NewItem::default()
                    },
                )?;
                Ok((p, t))
            })
            .await
            .unwrap();
        let waiting = db
            .read(move |c| items_tx(c, Basket::Thread(thread)))
            .await
            .unwrap();
        assert_eq!(
            waiting.iter().map(|i| (i.id, i.kind)).collect::<Vec<_>>(),
            [(p, ContextKind::Page), (t, ContextKind::Text)]
        );
        let empty = db
            .transaction(move |tx| add_tx(tx, Basket::Thread(thread), &NewItem::default()))
            .await;
        assert!(empty.is_err());
    }

    /// A note rides its item; it can be set, changed and cleared.
    #[tokio::test]
    async fn an_items_note_can_be_set_and_cleared() {
        let (db, thread, _) = with_threads().await;
        let id = db
            .transaction(move |tx| {
                add_tx(
                    tx,
                    Basket::Thread(thread),
                    &NewItem {
                        text: Some("fn a() {}".into()),
                        note: Some("handle the empty case".into()),
                        ..NewItem::default()
                    },
                )
            })
            .await
            .unwrap();
        let before = db
            .transaction(move |tx| set_note_tx(tx, id, Some("rename it")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.note.as_deref(), Some("handle the empty case"));
        db.transaction(move |tx| set_note_tx(tx, id, None))
            .await
            .unwrap();
        let item = db.read(move |c| get_tx(c, id)).await.unwrap().unwrap();
        assert_eq!(item.note, None);
    }

    /// A basket's name is trimmed, short, not the thread basket's, and
    /// unique in any case.
    #[tokio::test]
    async fn a_baskets_name_is_checked() {
        let (db, _, _) = with_threads().await;
        let made = db
            .transaction(|tx| create_basket_tx(tx, "  Auth research "))
            .await
            .unwrap();
        assert_eq!(made.name, "Auth research");
        for bad in [
            "",
            "   ",
            "this thread",
            &"x".repeat(NAME_MAX + 1),
            "AUTH RESEARCH",
        ] {
            let bad = bad.to_string();
            let refused = db.transaction(move |tx| create_basket_tx(tx, &bad)).await;
            assert!(refused.is_err());
        }
        let (again, created) = db
            .transaction(|tx| ensure_basket_tx(tx, "auth research"))
            .await
            .unwrap();
        assert_eq!((again.id, created), (made.id, false));
        let other = db
            .transaction(|tx| create_basket_tx(tx, "Release"))
            .await
            .unwrap();
        let taken = db
            .transaction(move |tx| rename_basket_tx(tx, other.id, "Auth Research"))
            .await;
        assert!(taken.is_err());
        let before = db
            .transaction(move |tx| rename_basket_tx(tx, other.id, "Release notes"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.name, "Release");
    }

    /// Clear, remove and move take only what they're asked for and say what
    /// it was; deleting a basket takes its items.
    #[tokio::test]
    async fn clear_remove_move_and_delete() {
        let (db, thread, _) = with_threads().await;
        let (basket, ids) = db
            .transaction(move |tx| {
                let basket = create_basket_tx(tx, "Auth")?.id;
                let mut ids = Vec::new();
                for n in 0..3 {
                    ids.push(add_tx(
                        tx,
                        Basket::Thread(thread),
                        &page(&format!("oxplow:oxplow/knowledge?id={n}&store=oxwiki")),
                    )?);
                }
                Ok((basket, ids))
            })
            .await
            .unwrap();
        let (first, second) = (ids[0], ids[1]);
        let removed = db
            .transaction(move |tx| remove_tx(tx, first))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            removed.source_uri.as_deref(),
            Some("oxplow:oxplow/knowledge?id=0&store=oxwiki")
        );
        let before = db
            .transaction(move |tx| move_tx(tx, second, Basket::Named(basket)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.basket(), Basket::Thread(thread));
        let cleared = db
            .transaction(move |tx| clear_tx(tx, Basket::Thread(thread)))
            .await
            .unwrap();
        assert_eq!(cleared.len(), 1);
        let in_basket = db
            .read(move |c| items_tx(c, Basket::Named(basket)))
            .await
            .unwrap();
        assert_eq!(in_basket.iter().map(|i| i.id).collect::<Vec<_>>(), [second]);
        let (gone, items) = db
            .transaction(move |tx| delete_basket_tx(tx, basket))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((gone.name.as_str(), items.len()), ("Auth", 1));
        assert!(db.read(move |c| get_tx(c, second)).await.unwrap().is_none());
    }

    /// A thread's next prompt carries its own items first, then each basket
    /// attached to it by name, with the stream each was collected in; a
    /// basket attached elsewhere, or to nothing, isn't carried.
    #[tokio::test]
    async fn a_threads_next_prompt_carries_its_own_and_its_attached_baskets() {
        let (db, thread, other) = with_threads().await;
        let (own, zed, alpha) = db
            .transaction(move |tx| {
                let zed = create_basket_tx(tx, "Zed")?.id;
                let alpha = create_basket_tx(tx, "alpha")?.id;
                let elsewhere = create_basket_tx(tx, "Elsewhere")?.id;
                let loose = create_basket_tx(tx, "Loose")?.id;
                let zed_item = add_tx(
                    tx,
                    Basket::Named(zed),
                    &NewItem {
                        stream_id: Some(2),
                        ..page("oxplow:oxplow/file?path=z.rs")
                    },
                )?;
                let alpha_item = add_tx(
                    tx,
                    Basket::Named(alpha),
                    &page("oxplow:oxplow/commit?sha=a"),
                )?;
                add_tx(
                    tx,
                    Basket::Named(elsewhere),
                    &page("oxplow:oxplow/commit?sha=e"),
                )?;
                add_tx(
                    tx,
                    Basket::Named(loose),
                    &page("oxplow:oxplow/commit?sha=l"),
                )?;
                let own = add_tx(
                    tx,
                    Basket::Thread(thread),
                    &NewItem {
                        stream_id: Some(1),
                        ..page("oxplow:oxplow/commit?sha=o")
                    },
                )?;
                attach_tx(tx, zed, Some(thread))?;
                attach_tx(tx, alpha, Some(thread))?;
                attach_tx(tx, elsewhere, Some(other))?;
                Ok((own, zed_item, alpha_item))
            })
            .await
            .unwrap();
        let waiting = db
            .read(move |c| waiting_for_thread_tx(c, thread))
            .await
            .unwrap();
        assert_eq!(
            waiting
                .iter()
                .map(|w| (w.item.id, w.basket.clone()))
                .collect::<Vec<_>>(),
            [
                (own, None),
                (alpha, Some("alpha".to_string())),
                (zed, Some("Zed".to_string())),
            ]
        );
        assert_eq!(
            waiting[2].stream,
            Some(ItemStream {
                id: 2,
                title: "Feature".into(),
                branch: "feature".into()
            })
        );
    }

    /// Delivery records the turn on the handoffs, links the turn to what it
    /// was given, deletes the items and detaches their baskets; an item
    /// added after the build stays, and with no turn open the items still go.
    #[tokio::test]
    async fn delivery_links_the_turn_deletes_the_items_and_detaches_their_baskets() {
        let (db, thread, _) = with_threads().await;
        let (basket, own, attached, later) = db
            .transaction(move |tx| {
                let basket = create_basket_tx(tx, "Auth")?.id;
                let own = add_tx(tx, Basket::Thread(thread), &page("oxplow:oxplow/commit?sha=abc1234"))?;
                let attached = add_tx(tx, Basket::Named(basket), &page("oxplow:oxplow/commit?sha=def5678"))?;
                attach_tx(tx, basket, Some(thread))?;
                tx.execute(
                    "INSERT INTO agent_turn (id, thread_id, prompt, started_at) VALUES (3, 1, 'p', 'a')",
                    [],
                )
                .map_err(map_sql_err)?;
                // Added after the prompt was built: not in what it carried.
                let later = add_tx(tx, Basket::Thread(thread), &page("oxplow:oxplow/commit?sha=later"))?;
                let handoff = oxplow_domain::HandoffId::new(7);
                take_delivered_tx(tx, thread, None, &[handoff], &[own, attached])?;
                Ok((basket, own, attached, later))
            })
            .await
            .unwrap();
        let (edges, attached_to): (Vec<String>, Option<i64>) = db
            .read(move |c| {
                let mut stmt = c
                    .prepare("SELECT target_uri FROM page_ref WHERE source_uri = 'oxplow:oxplow/turn?id=trn3' ORDER BY target_uri")
                    .map_err(map_sql_err)?;
                let edges = stmt
                    .query_map([], |r| r.get(0))
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok((edges, get_basket_tx(c, basket)?.unwrap().attached_thread_id))
            })
            .await
            .unwrap();
        assert_eq!(
            edges,
            [
                "oxplow:oxplow/commit?sha=abc1234".to_string(),
                "oxplow:oxplow/commit?sha=def5678".to_string(),
                oxplow_domain::uri::build::handoff_uri(oxplow_domain::HandoffId::new(7)),
            ]
        );
        assert_eq!(attached_to, None, "a basket is attached for one prompt");
        for gone in [own, attached] {
            assert!(db.read(move |c| get_tx(c, gone)).await.unwrap().is_none());
        }
        assert!(db.read(move |c| get_tx(c, later)).await.unwrap().is_some());

        // No turn open: nothing linked, the item still goes.
        let next = db
            .transaction(move |tx| {
                tx.execute("UPDATE agent_turn SET ended_at = 'b'", [])
                    .map_err(map_sql_err)?;
                take_delivered_tx(tx, thread, None, &[], &[later])?;
                Ok(later)
            })
            .await
            .unwrap();
        assert!(db.read(move |c| get_tx(c, next)).await.unwrap().is_none());
    }

    /// A thread's own items go with it; a basket attached to it is detached.
    #[tokio::test]
    async fn deleting_a_thread_takes_its_items_and_detaches_its_baskets() {
        let (db, thread, _) = with_threads().await;
        let (basket, own) = db
            .transaction(move |tx| {
                let basket = create_basket_tx(tx, "Auth")?.id;
                add_tx(
                    tx,
                    Basket::Named(basket),
                    &page("oxplow:oxplow/commit?sha=a"),
                )?;
                let own = add_tx(
                    tx,
                    Basket::Thread(thread),
                    &page("oxplow:oxplow/commit?sha=b"),
                )?;
                attach_tx(tx, basket, Some(thread))?;
                tx.execute("DELETE FROM threads WHERE id = 1", [])
                    .map_err(map_sql_err)?;
                Ok((basket, own))
            })
            .await
            .unwrap();
        assert!(db.read(move |c| get_tx(c, own)).await.unwrap().is_none());
        let kept = db
            .read(move |c| get_basket_tx(c, basket))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.attached_thread_id, None);
        assert_eq!(
            db.read(move |c| items_tx(c, Basket::Named(basket)))
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
