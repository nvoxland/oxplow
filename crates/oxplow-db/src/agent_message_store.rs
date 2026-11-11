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

/// How long after it was sent a send that asked to be told when its
/// recipient is idle may still be matched to that notice (the recipient may
/// work on it for a while).
pub const IDLE_MATCH_WINDOW_SECONDS: i64 = 3600;

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
    /// The call asked to be told when the recipient is idle.
    pub notify_when_idle: bool,
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
        notify_when_idle,
        at,
    } = *sent;
    conn.execute(
        "INSERT INTO agent_message (thread_id, agent_session_id, turn_id, call_id, recipient,
                                    text_hash, sent_at, notify_when_idle)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            thread.value(),
            session.map(|s| s.value()),
            turn.map(|t| t.value()),
            call,
            recipient,
            text_hash(text),
            ts_to_string(at),
            notify_when_idle
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
    find_tx(
        conn,
        Which::Text(&text_hash(text)),
        at,
        MATCH_WINDOW_SECONDS,
    )
}

/// The send an idle notice about session `name` received at `at` matches:
/// the one unmatched send to `name` that asked to be told when it is idle,
/// within [`IDLE_MATCH_WINDOW_SECONDS`] before it (same rules as
/// [`find_sent_tx`]: none, or more than one, is no match).
pub fn find_idle_ask_tx(
    conn: &Connection,
    name: &str,
    at: Timestamp,
) -> Result<Option<(i64, AgentTurnId)>, DomainError> {
    find_tx(conn, Which::IdleAsk(name), at, IDLE_MATCH_WINDOW_SECONDS)
}

/// What a match looks for: a send of this text hash not yet received, or a
/// send to this recipient that asked for an idle notice not yet answered.
enum Which<'a> {
    Text(&'a str),
    IdleAsk(&'a str),
}

fn find_tx(
    conn: &Connection,
    which: Which<'_>,
    at: Timestamp,
    window: i64,
) -> Result<Option<(i64, AgentTurnId)>, DomainError> {
    let (hash, recipient) = match which {
        Which::Text(hash) => (Some(hash), None),
        Which::IdleAsk(name) => (None, Some(name)),
    };
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.turn_id FROM agent_message m
              WHERE m.turn_id IS NOT NULL
                AND CASE WHEN ?4 IS NULL
                         THEN m.text_hash = ?1 AND m.received_turn_id IS NULL
                         ELSE m.recipient = ?4 AND m.notify_when_idle = 1
                              AND m.idle_notice_turn_id IS NULL END
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
        .query_map(params![hash, ts_to_string(at), window, recipient], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(map_sql_err)?
        .collect::<Result<_, _>>()
        .map_err(map_sql_err)?;
    Ok(match found[..] {
        [(id, turn)] => Some((id, AgentTurnId::new(turn))),
        _ => None,
    })
}

/// The idle notice that answers send `sent` joined turn `notice`: its ask
/// is spent.
pub fn mark_idle_notice_tx(
    conn: &Connection,
    sent: i64,
    notice: AgentTurnId,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE agent_message SET idle_notice_turn_id = ?2 WHERE id = ?1",
        params![sent, notice.value()],
    )
    .map_err(map_sql_err)?;
    Ok(())
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
                            notify_when_idle: false,
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

    /// V27 gives an ask for an idle notice its own columns: a row written
    /// with the old marker hash is folded into the send's row (the ask, and
    /// the turn the notice joined) and removed, so no marker is left.
    #[test]
    fn v27_folds_the_old_marker_rows_into_their_sends() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::embedded::migrations::runner()
            .set_target(refinery::Target::Version(26))
            .run(&mut conn)
            .unwrap();
        let marker = "oxplow:agent_message:notify_when_idle";
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys = OFF;
             INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/r', 'a', 'a');
             INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (1, 1, 't', 'a', 'a');
             INSERT INTO agent_turn (id, thread_id, prompt, started_at) VALUES (1, 1, 'p', 'a'), (2, 1, 'q', 'a');
             INSERT INTO agent_message (id, thread_id, turn_id, call_id, recipient, text_hash, sent_at, received_turn_id) VALUES
               (1, 1, 1, 'c1', 'asked', 'h1', 'a', NULL),
               (2, 1, 1, 'c1', 'asked', '{marker}', 'a', 2),
               (3, 1, 1, 'c2', 'waiting', 'h2', 'a', NULL),
               (4, 1, 1, 'c2', 'waiting', '{marker}', 'a', NULL),
               (5, 1, 1, 'c3', 'plain', 'h3', 'a', NULL);"
        ))
        .unwrap();
        crate::database::embedded::migrations::runner()
            .run(&mut conn)
            .unwrap();
        let rows: Vec<(i64, String, i64, Option<i64>)> = conn
            .prepare(
                "SELECT id, text_hash, notify_when_idle, idle_notice_turn_id
                   FROM agent_message ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (1, "h1".into(), 1, Some(2)),
                (3, "h2".into(), 1, None),
                (5, "h3".into(), 0, None),
            ]
        );
    }
}
