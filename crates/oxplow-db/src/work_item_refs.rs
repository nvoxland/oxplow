//! A work item's `page_ref` slices, restated from the work-item interface
//! (`work_item`, `work_item_link`, `work_item_comment`) — the same for
//! every list, so nothing here reads a list's own tables
//! (`.context/work-items.md`). The item is the source of all three: its
//! body's mentions, its links (`work_item_link:<type>`, the list's own
//! types) and its comments' mentions (`comment_*`). A deleted or unknown
//! item has none.

use rusqlite::OptionalExtension;

use oxplow_domain::refs::kind::KindRegistry;
use oxplow_domain::DomainError;

use crate::database::map_sql_err;
use crate::page_ref_projections::{
    work_item_body_ref_types, work_item_comment_edges, work_item_comment_ref_types,
    work_item_edges, work_item_link_edges, KIND_WORK_ITEM, RT_LINK_PREFIX,
};
use crate::page_ref_store::replace_source_for_ref_types_tx;

/// Restate `item_ref`'s body, link and comment slices from the interface,
/// in the caller's transaction.
pub fn restate_tx(
    conn: &rusqlite::Connection,
    kinds: &KindRegistry,
    item_ref: &str,
) -> Result<(), DomainError> {
    let id = item_ref
        .strip_prefix("work_item:")
        .ok_or_else(|| DomainError::Invalid(format!("`{item_ref}` is not a work item")))?;
    let live: Option<(String, String)> = conn
        .query_row(
            "SELECT title, body FROM work_item WHERE ref = ?1 AND deleted_at IS NULL",
            [item_ref],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    let (body, links, comments) = match &live {
        None => (Vec::new(), Vec::new(), Vec::new()),
        Some((title, body)) => {
            let links: Vec<(String, String)> = rows(
                conn,
                "SELECT to_ref, link_type FROM work_item_link WHERE from_ref = ?1
                 ORDER BY created_at, to_ref",
                item_ref,
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let comments: Vec<String> = rows(
                conn,
                "SELECT body FROM work_item_comment WHERE ref = ?1 ORDER BY created_at, id",
                item_ref,
                |r| r.get(0),
            )?;
            (
                work_item_edges(kinds, id, title, body),
                work_item_link_edges(id, &links),
                work_item_comment_edges(kinds, id, comments.iter().map(String::as_str)),
            )
        }
    };
    replace_source_for_ref_types_tx(conn, KIND_WORK_ITEM, id, &work_item_body_ref_types(), body)?;
    replace_source_for_ref_types_tx(
        conn,
        KIND_WORK_ITEM,
        id,
        &work_item_comment_ref_types(),
        comments,
    )?;
    // The link slice is every `work_item_link:` type the item has now or
    // had: a list names its own.
    let mut link_types: Vec<String> = rows(
        conn,
        "SELECT DISTINCT ref_type FROM page_ref
         WHERE source_kind = 'work_item' AND source_id = ?1
           AND substr(ref_type, 1, length('work_item_link:')) = 'work_item_link:'",
        id,
        |r| r.get(0),
    )?;
    for edge in &links {
        if !link_types.contains(&edge.ref_type) {
            link_types.push(edge.ref_type.clone());
        }
    }
    debug_assert!(link_types.iter().all(|t| t.starts_with(RT_LINK_PREFIX)));
    replace_source_for_ref_types_tx(conn, KIND_WORK_ITEM, id, &link_types, links)
}

/// Every item the interface holds, deleted ones included (restating one
/// clears its slices): what the boot repair restates.
pub fn all_refs_tx(conn: &rusqlite::Connection) -> Result<Vec<String>, DomainError> {
    let mut stmt = conn
        .prepare("SELECT ref FROM work_item ORDER BY ref")
        .map_err(map_sql_err)?;
    let refs = stmt
        .query_map([], |r| r.get(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<String>>>())
        .map_err(map_sql_err)?;
    Ok(refs)
}

fn rows<T>(
    conn: &rusqlite::Connection,
    sql: &str,
    param: &str,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, DomainError> {
    let mut stmt = conn.prepare(sql).map_err(map_sql_err)?;
    let out = stmt
        .query_map([param], map)
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<T>>>())
        .map_err(map_sql_err);
    out
}
