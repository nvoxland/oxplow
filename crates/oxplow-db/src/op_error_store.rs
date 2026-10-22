//! The state a person gives a failed operation (`v_op_error.seen`,
//! `dismissed`): marks in the event log, `ui.op_error.seen` and
//! `ui.op_error.dismissed`, each naming the `ui.op_failed` event it is
//! about. Read here inside a command's transaction
//! (`oxplow.op_error.*`); written as that command's events.

use oxplow_domain::DomainError;
use rusqlite::{params, Connection};

use crate::database::map_sql_err;

/// Whether `event` is a failed operation (a `ui.op_failed` event).
pub fn is_op_failed_tx(conn: &Connection, event: &str) -> Result<bool, DomainError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM event_log WHERE id = ?1 AND type = 'ui.op_failed')",
        params![event],
        |r| r.get(0),
    )
    .map_err(map_sql_err)
}

/// Whether the person has seen failed operation `event`, and whether they
/// dismissed it (dismissing is seeing).
pub fn state_tx(conn: &Connection, event: &str) -> Result<(bool, bool), DomainError> {
    conn.query_row(
        "SELECT
           EXISTS (SELECT 1 FROM event_log WHERE type IN ('ui.op_error.seen', 'ui.op_error.dismissed')
                     AND json_extract(payload, '$.event') = ?1),
           EXISTS (SELECT 1 FROM event_log WHERE type = 'ui.op_error.dismissed'
                     AND json_extract(payload, '$.event') = ?1)",
        params![event],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(map_sql_err)
}

/// The failed operations the person hasn't dismissed, oldest first.
pub fn undismissed_tx(conn: &Connection) -> Result<Vec<String>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT f.id FROM event_log f
              WHERE f.type = 'ui.op_failed'
                AND NOT EXISTS (SELECT 1 FROM event_log d WHERE d.type = 'ui.op_error.dismissed'
                                  AND json_extract(d.payload, '$.event') = f.id)
              ORDER BY f.seq",
        )
        .map_err(map_sql_err)?;
    let ids = st
        .query_map([], |r| r.get(0))
        .map_err(map_sql_err)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(map_sql_err)?;
    Ok(ids)
}
