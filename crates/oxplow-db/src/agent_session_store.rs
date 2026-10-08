//! `agent_session` rows: the agent slots a person opened on a thread
//! (`.context/data-model.md` "agent_session").

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};

use oxplow_domain::agent_session::{
    AgentSession, NewAgentSession, SessionCloseReason, SessionKind,
};
use oxplow_domain::stores::AgentSessionStore;
use oxplow_domain::{AgentSessionId, DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, string_to_ts, ts_to_string, Database};
use oxplow_domain::vocabulary::VocabularyHandle;

const COLUMNS: &str = "id, thread_id, kind, harness, acp_agent, title, resume_session_id, host,
                       opened_at, closed_at, closed_reason, updated_at";

fn conv(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

fn invalid(what: &str, value: &str) -> rusqlite::Error {
    conv(DomainError::Storage(format!(
        "unknown agent_session {what}: {value}"
    )))
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    let kind: String = row.get("kind")?;
    let harness: String = row.get("harness")?;
    let opened_at: String = row.get("opened_at")?;
    let closed_at: Option<String> = row.get("closed_at")?;
    let closed_reason: Option<String> = row.get("closed_reason")?;
    let updated_at: String = row.get("updated_at")?;
    Ok(AgentSession {
        id: AgentSessionId::new(row.get("id")?),
        thread_id: ThreadId::new(row.get("thread_id")?),
        kind: SessionKind::parse(&kind).ok_or_else(|| invalid("kind", &kind))?,
        harness,
        acp_agent: row.get("acp_agent")?,
        title: row.get("title")?,
        resume_session_id: row.get("resume_session_id")?,
        host: row.get("host")?,
        opened_at: string_to_ts(&opened_at).map_err(conv)?,
        closed_at: closed_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(conv)?,
        closed_reason: closed_reason
            .map(|r| SessionCloseReason::parse(&r).ok_or_else(|| invalid("closed_reason", &r)))
            .transpose()?,
        updated_at: string_to_ts(&updated_at).map_err(conv)?,
    })
}

fn query(
    conn: &Connection,
    filter: &str,
    args: impl rusqlite::Params,
) -> Result<Vec<AgentSession>, DomainError> {
    let mut stmt = conn
        .prepare(&format!("SELECT {COLUMNS} FROM agent_session {filter}"))
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(args, row_to_session)
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(map_sql_err)?;
    Ok(rows)
}

pub fn get_tx(conn: &Connection, id: AgentSessionId) -> Result<Option<AgentSession>, DomainError> {
    Ok(query(conn, "WHERE id = ?1", [id.value()])?.pop())
}

/// The thread's open sessions, oldest first: the order their tabs sit in.
pub fn list_open_for_thread_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Vec<AgentSession>, DomainError> {
    query(
        conn,
        "WHERE thread_id = ?1 AND closed_at IS NULL ORDER BY opened_at, id",
        [thread.value()],
    )
}

/// The thread's most recently opened session that is still open.
pub fn newest_open_for_thread_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Option<AgentSession>, DomainError> {
    Ok(query(
        conn,
        "WHERE thread_id = ?1 AND closed_at IS NULL ORDER BY opened_at DESC, id DESC LIMIT 1",
        [thread.value()],
    )?
    .pop())
}

/// The thread's most recently opened session, open or not.
pub fn newest_for_thread_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Option<AgentSession>, DomainError> {
    Ok(query(
        conn,
        "WHERE thread_id = ?1 ORDER BY opened_at DESC, id DESC LIMIT 1",
        [thread.value()],
    )?
    .pop())
}

/// The agent session a report on `thread` came from, by what the sender
/// said, in order: the session it named (`X-Oxplow-Session`; refused when
/// it is another thread's), the session whose resume id is the harness's
/// session id `harness_session` (an open one first, then the newest), the
/// thread's newest open session (one with an open turn first; logged,
/// since every process oxplow starts names its session), or none — a report
/// from an agent oxplow didn't start.
pub fn resolve_tx(
    conn: &Connection,
    thread: ThreadId,
    named: Option<AgentSessionId>,
    harness_session: Option<&str>,
) -> Result<Option<AgentSession>, DomainError> {
    if let Some(id) = named {
        match get_tx(conn, id)? {
            Some(s) if s.thread_id == thread => return Ok(Some(s)),
            _ => {
                tracing::warn!(%id, %thread, "a report named a session not on its thread; ignored")
            }
        }
    }
    if let Some(sid) = harness_session.filter(|s| !s.is_empty()) {
        if let Some(s) = query(
            conn,
            "WHERE thread_id = ?1 AND resume_session_id = ?2
             ORDER BY closed_at IS NULL DESC, opened_at DESC, id DESC LIMIT 1",
            params![thread.value(), sid],
        )?
        .pop()
        {
            return Ok(Some(s));
        }
    }
    let newest = query(
        conn,
        "WHERE thread_id = ?1 AND closed_at IS NULL
         ORDER BY EXISTS (SELECT 1 FROM agent_turn t
                           WHERE t.agent_session_id = agent_session.id AND t.ended_at IS NULL) DESC,
                  opened_at DESC, id DESC
         LIMIT 1",
        [thread.value()],
    )?
    .pop();
    if let Some(s) = &newest {
        tracing::warn!(session = %s.id, %thread, "a report named no session; took the thread's newest");
    }
    Ok(newest)
}

/// Open a session on its thread at `now`.
pub fn insert_tx(
    conn: &Connection,
    new: &NewAgentSession,
    now: Timestamp,
) -> Result<AgentSession, DomainError> {
    let at = ts_to_string(now);
    let id = conn
        .query_row(
            "INSERT INTO agent_session (thread_id, kind, harness, acp_agent, title, opened_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) RETURNING id",
            params![
                new.thread_id.value(),
                new.kind.as_str(),
                new.harness.as_str(),
                new.acp_agent,
                new.title,
                at
            ],
            |r| r.get::<_, i64>(0),
        )
        .map_err(map_sql_err)?;
    get_tx(conn, AgentSessionId::new(id))?
        .ok_or_else(|| DomainError::Storage("agent_session vanished after insert".into()))
}

/// Close an open session. `false` when it was already closed (or gone).
pub fn close_tx(
    conn: &Connection,
    id: AgentSessionId,
    reason: SessionCloseReason,
    now: Timestamp,
) -> Result<bool, DomainError> {
    let at = ts_to_string(now);
    let n = conn
        .execute(
            "UPDATE agent_session SET closed_at = ?2, closed_reason = ?3, updated_at = ?2
              WHERE id = ?1 AND closed_at IS NULL",
            params![id.value(), at, reason.as_str()],
        )
        .map_err(map_sql_err)?;
    Ok(n == 1)
}

/// Record the harness's own session id, to resume it later.
pub fn set_resume_tx(
    conn: &Connection,
    id: AgentSessionId,
    resume_session_id: &str,
    now: Timestamp,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE agent_session SET resume_session_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id.value(), resume_session_id, ts_to_string(now)],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Forget the session's resume id when it is still `resume_session_id` —
/// that harness session is gone (cleared, or its transcript missing); a
/// newer one that replaced it stays. Whether it was forgotten.
pub fn forget_resume_tx(
    conn: &Connection,
    id: AgentSessionId,
    resume_session_id: &str,
    now: Timestamp,
) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            "UPDATE agent_session SET resume_session_id = '', updated_at = ?3
              WHERE id = ?1 AND resume_session_id = ?2",
            params![id.value(), resume_session_id, ts_to_string(now)],
        )
        .map_err(map_sql_err)?;
    Ok(n == 1)
}

/// Rename a session; returns the title it had, `None` when it's gone.
pub fn set_title_tx(
    conn: &Connection,
    id: AgentSessionId,
    title: &str,
    now: Timestamp,
) -> Result<Option<String>, DomainError> {
    let before: Option<String> = conn
        .query_row(
            "SELECT title FROM agent_session WHERE id = ?1",
            [id.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    if before.is_some() {
        conn.execute(
            "UPDATE agent_session SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.value(), title, ts_to_string(now)],
        )
        .map_err(map_sql_err)?;
    }
    Ok(before)
}

#[derive(Clone)]
pub struct SqliteAgentSessionStore {
    db: Database,
    vocabulary: VocabularyHandle,
}

impl SqliteAgentSessionStore {
    /// A store with its own core schema registry; `Services` shares one
    /// via [`Self::with_vocabulary`].
    pub fn new(db: Database) -> Self {
        Self::with_vocabulary(db, VocabularyHandle::core())
    }

    pub fn with_vocabulary(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self { db, vocabulary }
    }

    /// Close every open session of `thread` (`reason`) in one transaction —
    /// their open turns end and each logs `stopped` — and return them: their
    /// processes are the caller's to stop.
    pub async fn close_for_thread(
        &self,
        thread: ThreadId,
        reason: SessionCloseReason,
    ) -> Result<Vec<AgentSessionId>, DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let ev = crate::event_log_store::EventCtx::system(&vocabulary, "agent_sessions");
                let now = Timestamp::now();
                let mut closed = Vec::new();
                for s in list_open_for_thread_tx(tx, thread)? {
                    if crate::agent_stores::close_session_tx(tx, &ev, s.id, reason, now)? {
                        closed.push(s.id);
                    }
                }
                Ok(closed)
            })
            .await
    }

    /// The thread's most recently opened session, open or not.
    pub async fn newest_for_thread(
        &self,
        thread: ThreadId,
    ) -> Result<Option<AgentSession>, DomainError> {
        self.db
            .read(move |conn| newest_for_thread_tx(conn, thread))
            .await
    }
}

#[async_trait]
impl AgentSessionStore for SqliteAgentSessionStore {
    async fn get(&self, id: &AgentSessionId) -> Result<Option<AgentSession>, DomainError> {
        let id = *id;
        self.db.read(move |conn| get_tx(conn, id)).await
    }

    async fn list_open_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Vec<AgentSession>, DomainError> {
        let thread = *thread;
        self.db
            .read(move |conn| list_open_for_thread_tx(conn, thread))
            .await
    }

    async fn open(&self, new: &NewAgentSession) -> Result<AgentSession, DomainError> {
        let new = new.clone();
        self.db
            .transaction(move |tx| insert_tx(tx, &new, Timestamp::now()))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000 + ms)
    }

    fn fixture() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        crate::database::migrate_and_compile(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'refs/heads/main', 'main', '/r', 't', 't');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (1, 1, 'a', 'active', 't', 't'), (2, 1, 'b', 'queued', 't', 't');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_session_round_trips_and_closes_once() {
        let conn = fixture();
        let opened = insert_tx(
            &conn,
            &NewAgentSession::chat(ThreadId::new(1), "acp", "gemini"),
            at(1),
        )
        .unwrap();
        assert_eq!(opened.kind, SessionKind::Chat);
        assert_eq!(opened.acp_agent.as_deref(), Some("gemini"));
        assert_eq!((opened.opened_at, opened.updated_at), (at(1), at(1)));
        assert!(opened.is_open());
        assert_eq!(get_tx(&conn, opened.id).unwrap(), Some(opened.clone()));

        set_resume_tx(&conn, opened.id, "r9", at(2)).unwrap();
        // Forgetting a resume id that was replaced leaves the newer one.
        assert!(!forget_resume_tx(&conn, opened.id, "r8", at(2)).unwrap());
        assert_eq!(
            set_title_tx(&conn, opened.id, "review", at(3)).unwrap(),
            Some(String::new())
        );
        assert!(close_tx(&conn, opened.id, SessionCloseReason::Closed, at(4)).unwrap());
        assert!(!close_tx(&conn, opened.id, SessionCloseReason::ThreadClosed, at(5)).unwrap());

        let back = get_tx(&conn, opened.id).unwrap().unwrap();
        assert_eq!(back.resume_session_id, "r9");
        assert_eq!(back.title, "review");
        assert_eq!(back.closed_at, Some(at(4)));
        assert_eq!(back.closed_reason, Some(SessionCloseReason::Closed));
        assert_eq!(back.updated_at, at(4));
    }

    #[test]
    fn a_thread_lists_its_open_sessions_oldest_first() {
        let conn = fixture();
        let thread = ThreadId::new(1);
        let open = |ms, harness: &str| {
            insert_tx(&conn, &NewAgentSession::terminal(thread, harness), at(ms))
                .unwrap()
                .id
        };
        let a = open(1, "claude");
        let b = open(2, "codex");
        let c = open(3, "claude");
        insert_tx(
            &conn,
            &NewAgentSession::terminal(ThreadId::new(2), "claude"),
            at(4),
        )
        .unwrap();
        close_tx(&conn, c, SessionCloseReason::Closed, at(5)).unwrap();

        let ids = |v: Vec<AgentSession>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(
            ids(list_open_for_thread_tx(&conn, thread).unwrap()),
            vec![a, b]
        );
        assert_eq!(
            newest_open_for_thread_tx(&conn, thread)
                .unwrap()
                .map(|s| s.id),
            Some(b)
        );
        assert_eq!(
            newest_for_thread_tx(&conn, thread).unwrap().map(|s| s.id),
            Some(c)
        );
    }

    /// A harness is a key, stored as given: any one an extension declares.
    #[test]
    fn any_harness_key_round_trips() {
        let conn = fixture();
        for new in [
            NewAgentSession::terminal(ThreadId::new(1), "codex"),
            NewAgentSession::terminal(ThreadId::new(1), "someones-harness"),
            NewAgentSession::chat(ThreadId::new(1), "acp", "gemini"),
        ] {
            let s = insert_tx(&conn, &new, at(1)).unwrap();
            let back = get_tx(&conn, s.id).unwrap().unwrap();
            assert_eq!(
                (back.kind, back.harness, back.acp_agent),
                (new.kind, new.harness, new.acp_agent)
            );
        }
    }

    #[test]
    fn closing_the_thread_row_removes_its_sessions() {
        let conn = fixture();
        let s = insert_tx(
            &conn,
            &NewAgentSession::terminal(ThreadId::new(2), "claude"),
            at(1),
        )
        .unwrap();
        conn.execute("DELETE FROM threads WHERE id = 2", [])
            .unwrap();
        assert_eq!(get_tx(&conn, s.id).unwrap(), None);
    }
}
