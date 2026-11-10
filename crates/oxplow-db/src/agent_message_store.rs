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

/// A send: a call in `turn` of agent session `session` on `thread` sent
/// `text` to `recipient` at `at`, by tool call `call`.
pub struct Sent<'a> {
    pub thread: ThreadId,
    pub session: Option<AgentSessionId>,
    pub turn: Option<AgentTurnId>,
    pub call: Option<&'a str>,
    pub recipient: &'a str,
    pub text: &'a str,
    pub at: Timestamp,
}

/// Record a send.
pub fn record_sent_tx(conn: &Connection, sent: &Sent<'_>) -> Result<(), DomainError> {
    let Sent {
        thread,
        session,
        turn,
        call,
        recipient,
        text,
        at,
    } = *sent;
    conn.execute(
        "INSERT INTO agent_message (thread_id, agent_session_id, turn_id, call_id, recipient,
                                    text_hash, sent_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            thread.value(),
            session.map(|s| s.value()),
            turn.map(|t| t.value()),
            call,
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
/// it, as `(its row, the turn that sent it)`. A send whose call is known
/// (its row in `agent_tool_call`) to have been denied, abandoned or failed
/// never reached anyone, and is no candidate; one whose call is not
/// projected yet or still runs is. None when no send matches, or more than
/// one does (the match must be unambiguous).
pub fn find_sent_tx(
    conn: &Connection,
    text: &str,
    at: Timestamp,
) -> Result<Option<(i64, AgentTurnId)>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.turn_id FROM agent_message m
              WHERE m.text_hash = ?1 AND m.received_turn_id IS NULL AND m.turn_id IS NOT NULL
                AND julianday(m.sent_at) <= julianday(?2)
                AND julianday(m.sent_at) >= julianday(?2) - ?3 / 86400.0
                AND NOT EXISTS (
                      SELECT 1 FROM agent_tool_call c
                       WHERE c.call_id = m.call_id AND c.agent_session_id IS m.agent_session_id
                         AND (c.state IN ('denied', 'abandoned') OR c.ok = 0))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// A send whose call was denied, abandoned or failed never reached the
    /// receiver: it is no candidate, and a second send of the same text
    /// that did run then matches alone.
    #[tokio::test]
    async fn a_send_that_never_ran_is_not_a_candidate() {
        let db = Database::in_memory();
        let at = Timestamp::from_unix_ms(1_000_000);
        let found = db
            .transaction(move |tx| {
                tx.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                       VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/p', 'a', 'a');
                     INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (1, 1, 'T', 'a', 'a');
                     INSERT INTO agent_turn (id, thread_id, prompt, started_at) VALUES (1, 1, 'p', 'a');
                     INSERT INTO agent_tool_call (thread_id, turn_id, tool, at, call_id, state)
                       VALUES (1, 1, 'SendMessage', 'a', 'denied-call', 'denied');",
                )
                .map_err(map_sql_err)?;
                let send = |call: &str| {
                    record_sent_tx(
                        tx,
                        &Sent {
                            thread: ThreadId::new(1),
                            session: None,
                            turn: Some(AgentTurnId::new(1)),
                            call: Some(call),
                            recipient: "main",
                            text: "hello",
                            at: Timestamp::from_unix_ms(999_000),
                        },
                    )
                };
                send("denied-call")?;
                let alone = find_sent_tx(tx, "hello", at)?;
                // A second send of the same text, not (yet) projected: the
                // only candidate, so the match is unambiguous.
                send("ran-call")?;
                let after = find_sent_tx(tx, "hello", at)?;
                Ok((alone, after))
            })
            .await
            .unwrap();
        assert_eq!(found.0, None, "the denied send alone is no match");
        assert!(found.1.is_some(), "the send that ran matches alone");
    }
}
