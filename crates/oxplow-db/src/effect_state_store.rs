//! `effect_state` (V148, P8.D9): where each approved effect starts
//! reading the event log. The app's `effects` module sets it on a
//! person's approval and gates every run on it.

use rusqlite::{params, Connection, OptionalExtension};

use crate::database::map_sql_err;
use oxplow_domain::DomainError;

/// Start `effect` after the log's head as of now (a person just approved
/// it); returns that seq.
pub fn start_at_head_tx(conn: &Connection, effect: &str, now: &str) -> Result<i64, DomainError> {
    let head: i64 = conn
        .query_row("SELECT coalesce(max(seq), 0) FROM event_log", [], |r| {
            r.get(0)
        })
        .map_err(map_sql_err)?;
    conn.execute(
        "INSERT INTO effect_state (effect, start_after_seq, approved_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (effect) DO UPDATE SET
             start_after_seq = excluded.start_after_seq,
             approved_at = excluded.approved_at",
        params![effect, head, now],
    )
    .map_err(map_sql_err)?;
    Ok(head)
}

/// Where `effect` starts: the seq after which it reads; `None` before it
/// was ever approved.
pub fn start_after_tx(conn: &Connection, effect: &str) -> Result<Option<i64>, DomainError> {
    conn.query_row(
        "SELECT start_after_seq FROM effect_state WHERE effect = ?1",
        [effect],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sql_err)
}
