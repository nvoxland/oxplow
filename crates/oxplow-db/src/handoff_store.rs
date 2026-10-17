//! Handoffs: work handed from one thread to another (`oxplow.handoff.send`,
//! `oxplow.handoff.settle`). A handoff is a record oxplow keeps, never a
//! message it delivers: the recipient thread's agent is given it at the
//! start of its next prompt ([`SqliteHandoffStore::peek_for_prompt`], then
//! [`SqliteHandoffStore::mark_delivered`] once the agent has it), and the person sees it at once. Its body's
//! `[[…]]` links are its `page_ref` edges (source kind `handoff`). See
//! `.context/agent-model.md` "Handoffs and native messaging".

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use oxplow_domain::events::schema::{HandoffSent, HandoffSentV1, HandoffSettled, HandoffSettledV1};
use oxplow_domain::refs::build::{handoff_ref, thread_ref};
use oxplow_domain::refs::kind::{KindRegistry, HANDOFF};
use oxplow_domain::{AgentSessionId, DomainError, Envelope, HandoffId, ThreadId, Timestamp};

use crate::database::{map_sql_err, string_to_ts, ts_to_string, Database};
use crate::page_ref_projections::note_edges;
use crate::page_ref_store::replace_source_tx;

/// Where a handoff stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    Open,
    Taken,
    Declined,
    Withdrawn,
}

impl HandoffState {
    pub fn as_str(self) -> &'static str {
        match self {
            HandoffState::Open => "open",
            HandoffState::Taken => "taken",
            HandoffState::Declined => "declined",
            HandoffState::Withdrawn => "withdrawn",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => HandoffState::Open,
            "taken" => HandoffState::Taken,
            "declined" => HandoffState::Declined,
            "withdrawn" => HandoffState::Withdrawn,
            _ => return None,
        })
    }
}

/// A handoff as stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Handoff {
    pub id: HandoffId,
    pub from_thread_id: ThreadId,
    pub from_session_id: Option<AgentSessionId>,
    /// `user`, `agent`, `effect`, `job` or `system`.
    pub from_actor: String,
    pub to_thread_id: ThreadId,
    pub work_item: Option<String>,
    pub body: String,
    pub sent_at: Timestamp,
    pub delivered_at: Option<Timestamp>,
    pub delivered_to_session_id: Option<AgentSessionId>,
    pub state: HandoffState,
    pub settled_at: Option<Timestamp>,
    pub settled_by_session_id: Option<AgentSessionId>,
    pub note: Option<String>,
}

/// What `oxplow.handoff.send` records.
pub struct NewHandoff<'a> {
    pub from_thread: ThreadId,
    pub from_session: Option<AgentSessionId>,
    pub from_actor: &'a str,
    pub to_thread: ThreadId,
    pub work_item: Option<&'a str>,
    pub body: &'a str,
}

const SOURCE: &str = "system:handoffs";

fn row_to_handoff(row: &rusqlite::Row<'_>) -> rusqlite::Result<Handoff> {
    let ts = |col: &str| -> rusqlite::Result<Option<Timestamp>> {
        row.get::<_, Option<String>>(col)?
            .map(|s| {
                string_to_ts(&s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })
            .transpose()
    };
    let session = |col: &str| -> rusqlite::Result<Option<AgentSessionId>> {
        Ok(row.get::<_, Option<i64>>(col)?.map(AgentSessionId::new))
    };
    let state: String = row.get("state")?;
    Ok(Handoff {
        id: HandoffId::new(row.get("id")?),
        from_thread_id: ThreadId::new(row.get("from_thread_id")?),
        from_session_id: session("from_session_id")?,
        from_actor: row.get("from_actor")?,
        to_thread_id: ThreadId::new(row.get("to_thread_id")?),
        work_item: row.get("work_item")?,
        body: row.get("body")?,
        sent_at: ts("sent_at")?.unwrap_or_else(Timestamp::now),
        delivered_at: ts("delivered_at")?,
        delivered_to_session_id: session("delivered_to_session_id")?,
        state: HandoffState::parse(&state).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown handoff state `{state}`").into(),
            )
        })?,
        settled_at: ts("settled_at")?,
        settled_by_session_id: session("settled_by_session_id")?,
        note: row.get("note")?,
    })
}

/// Handoff `id`, on `conn`.
pub fn handoff_tx(
    conn: &rusqlite::Connection,
    id: HandoffId,
) -> Result<Option<Handoff>, DomainError> {
    conn.query_row(
        "SELECT * FROM handoff WHERE id = ?1",
        params![id.value()],
        row_to_handoff,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Record a handoff and its `page_ref` edges, on `conn` (a command's
/// transaction). Returns it and the `handoff.sent` to log.
pub fn send_tx(
    conn: &rusqlite::Connection,
    kinds: &KindRegistry,
    new: NewHandoff<'_>,
) -> Result<(Handoff, Envelope), DomainError> {
    let now = Timestamp::now();
    conn.execute(
        "INSERT INTO handoff (from_thread_id, from_session_id, from_actor, to_thread_id,
                              work_item, body, sent_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.from_thread.value(),
            new.from_session.map(|s| s.value()),
            new.from_actor,
            new.to_thread.value(),
            new.work_item,
            new.body,
            ts_to_string(now),
        ],
    )
    .map_err(map_sql_err)?;
    let id = HandoffId::new(conn.last_insert_rowid());
    replace_source_tx(
        conn,
        HANDOFF,
        &id.to_string(),
        note_edges(kinds, HANDOFF, &id.to_string(), new.body),
    )?;
    let handoff = handoff_tx(conn, id)?.ok_or_else(|| {
        DomainError::Invalid(format!("handoff `{id}` vanished in its own transaction"))
    })?;
    let event = Envelope::typed::<HandoffSent>(
        SOURCE,
        &HandoffSentV1 {
            handoff: handoff_ref(id),
            from_thread: thread_ref(new.from_thread),
            to_thread: thread_ref(new.to_thread),
            work_item: new.work_item.map(str::to_string),
        },
    )
    .with_subject(subject(&handoff));
    Ok((handoff, event))
}

/// Move handoff `id` to `state`, stamped by `session` with `note`, on
/// `conn`; `open` clears how it ended (an undo). Returns the
/// `handoff.settled` to log.
pub fn settle_tx(
    conn: &rusqlite::Connection,
    id: HandoffId,
    state: HandoffState,
    session: Option<AgentSessionId>,
    note: Option<&str>,
) -> Result<Envelope, DomainError> {
    let settled_at = (state != HandoffState::Open).then(|| ts_to_string(Timestamp::now()));
    let session = session.filter(|_| state != HandoffState::Open);
    let note = note.filter(|_| state != HandoffState::Open);
    conn.execute(
        "UPDATE handoff SET state = ?2, settled_at = ?3, settled_by_session_id = ?4, note = ?5
          WHERE id = ?1",
        params![
            id.value(),
            state.as_str(),
            settled_at,
            session.map(|s| s.value()),
            note
        ],
    )
    .map_err(map_sql_err)?;
    let handoff =
        handoff_tx(conn, id)?.ok_or_else(|| DomainError::Invalid(format!("no handoff `{id}`")))?;
    Ok(Envelope::typed::<HandoffSettled>(
        SOURCE,
        &HandoffSettledV1 {
            handoff: handoff_ref(id),
            state: state.as_str().into(),
            from_thread: thread_ref(handoff.from_thread_id),
            to_thread: thread_ref(handoff.to_thread_id),
        },
    )
    .with_subject(subject(&handoff)))
}

fn subject(h: &Handoff) -> Vec<String> {
    let mut subject = vec![
        handoff_ref(h.id),
        thread_ref(h.from_thread_id),
        thread_ref(h.to_thread_id),
    ];
    subject.extend(h.work_item.clone());
    subject
}

#[derive(Clone)]
pub struct SqliteHandoffStore {
    db: Database,
}

impl SqliteHandoffStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The open handoffs to `thread` no agent was given yet, oldest first,
    /// still undelivered: the delivery can fail, so the stamp
    /// ([`Self::mark_delivered`]) waits until the agent has them.
    pub async fn peek_for_prompt(&self, thread: ThreadId) -> Result<Vec<Handoff>, DomainError> {
        self.db
            .transaction(move |conn| {
                conn.prepare(
                    "SELECT * FROM handoff
                      WHERE to_thread_id = ?1 AND state = 'open' AND delivered_at IS NULL
                      ORDER BY sent_at, id",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![thread.value()], row_to_handoff)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_err(map_sql_err)
            })
            .await
    }

    /// Stamp handoffs `ids` delivered to `session` (those still waiting):
    /// each reaches the thread's agents once, at the first prompt that
    /// carries it.
    pub async fn mark_delivered(
        &self,
        ids: Vec<HandoffId>,
        session: Option<AgentSessionId>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                let now = ts_to_string(Timestamp::now());
                for id in ids.iter() {
                    tx.execute(
                        "UPDATE handoff SET delivered_at = ?2, delivered_to_session_id = ?3
                          WHERE id = ?1 AND delivered_at IS NULL",
                        params![id.value(), now, session.map(|s| s.value())],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Every handoff's id and body, for the boot-time `page_ref` backfill.
    pub async fn list_all_for_backfill(&self) -> Result<Vec<(String, String)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT id, body FROM handoff")?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        HandoffId::new(r.get::<_, i64>(0)?).to_string(),
                        r.get::<_, String>(1)?,
                    ))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}
