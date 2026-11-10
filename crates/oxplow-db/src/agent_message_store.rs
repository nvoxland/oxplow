//! Messages one agent session sends another (`agent_message`), recorded
//! when the sending call is requested and matched to the turn that receives
//! them. The harness gives no message id, so the match is oxplow's: the
//! same text, sent recently, by exactly one call. See
//! `.context/agent-model.md` "Handoffs and native messaging".

use rusqlite::{params, Connection};

use oxplow_domain::{AgentSessionId, AgentTurnId, DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// How long after it was sent a received message may still match its send.
pub const MATCH_WINDOW_SECONDS: i64 = 600;

/// The content hash two ends of a message are matched on.
pub fn text_hash(text: &str) -> String {
    crate::event_content_store::hash(text.trim().as_bytes())
}

/// A call in `turn` of agent session `session` on `thread` sent `text` to
/// `recipient` at `at`.
pub fn record_sent_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    turn: Option<AgentTurnId>,
    recipient: &str,
    text: &str,
    at: Timestamp,
) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO agent_message (thread_id, agent_session_id, turn_id, recipient, text_hash,
                                    sent_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            thread.value(),
            session.map(|s| s.value()),
            turn.map(|t| t.value()),
            recipient,
            text_hash(text),
            ts_to_string(at)
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// The send a message received at `at` with `text` matches: the one
/// unmatched send of the same text within [`MATCH_WINDOW_SECONDS`] before
/// it, as `(its row, the turn that sent it)`. None when no send matches, or
/// more than one does (the match must be unambiguous).
pub fn find_sent_tx(
    conn: &Connection,
    text: &str,
    at: Timestamp,
) -> Result<Option<(i64, AgentTurnId)>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id FROM agent_message
              WHERE text_hash = ?1 AND received_turn_id IS NULL AND turn_id IS NOT NULL
                AND julianday(sent_at) <= julianday(?2)
                AND julianday(sent_at) >= julianday(?2) - ?3 / 86400.0
              LIMIT 2",
        )
        .map_err(map_sql_err)?;
    let found: Vec<(i64, i64)> = stmt
        .query_map(
            params![text_hash(text), ts_to_string(at), MATCH_WINDOW_SECONDS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(map_sql_err)?
        .collect::<Result<_, _>>()
        .map_err(map_sql_err)?;
    Ok(match found[..] {
        [(id, turn)] => Some((id, AgentTurnId::new(turn))),
        _ => None,
    })
}

/// Send `sent` was received in turn `received`: a match is spent.
pub fn mark_received_tx(
    conn: &Connection,
    sent: i64,
    received: AgentTurnId,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE agent_message SET received_turn_id = ?2 WHERE id = ?1",
        params![sent, received.value()],
    )
    .map_err(map_sql_err)?;
    Ok(())
}
