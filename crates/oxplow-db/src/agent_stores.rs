//! Agent turns (`agent_turn` rows and their `agent.turn.*` events) and
//! agent status, which is read from the log: a thread's status is its
//! newest `agent.status.changed`.

use oxplow_domain::vocabulary::{Vocabulary, VocabularyHandle};

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};

use oxplow_domain::events::schema::{
    AgentStatusChanged, AgentTurnEnded, AgentTurnEndedV2, AgentTurnStarted, AgentTurnStartedV1,
    EventType,
};
use oxplow_domain::events::Anchors;
use oxplow_domain::hook::TurnOutcome;
use oxplow_domain::refs::build::{thread_ref, turn_ref};
use oxplow_domain::stores::{AgentStatusStore, AgentTurnStore};
use oxplow_domain::{
    AgentStatus, AgentTurn, AgentTurnId, DomainError, StreamId, ThreadId, Timestamp,
};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::{anchors_for_thread_tx, EventCtx};

fn map_err_text(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

// -- AgentTurn ---------------------------------------------------------

fn row_to_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTurn> {
    let id: i64 = row.get("id")?;
    let thread_id: i64 = row.get("thread_id")?;
    let prompt: String = row.get("prompt")?;
    let answer: Option<String> = row.get("answer")?;
    let session_id: Option<String> = row.get("session_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    Ok(AgentTurn {
        id: AgentTurnId::new(id),
        thread_id: ThreadId::new(thread_id),
        prompt,
        answer,
        session_id,
        started_at: string_to_ts(&started_at).map_err(map_err_text)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err_text)?,
        start_snapshot_id: row.get("start_snapshot_id")?,
        snapshot_id: row.get("snapshot_id")?,
    })
}

/// The anchors agent activity on `thread` carries (P3.3): the thread, its
/// stream, the thread's open turn and its single open effort (none when
/// two or more are open — attribution never guesses).
pub fn activity_anchors_tx(conn: &Connection, thread: ThreadId) -> Result<Anchors, DomainError> {
    let mut anchors = anchors_for_thread_tx(conn, thread)?;
    anchors.turn_id = open_turn_ids_tx(conn, thread)?.first().map(|t| t.value());
    anchors.effort_id =
        crate::effort_store::find_single_open_for_thread_tx(conn, thread)?.map(|e| e.id);
    Ok(anchors)
}

/// The turn on `thread` that something measured at `at` happened in: the
/// newest one started at or before `at` — the one whose span holds it, or
/// the last before it when it fell between turns. `None` before the
/// thread's first turn. What places a report that arrives after its turn
/// ended (an agent's telemetry export).
pub fn turn_at_tx(
    conn: &Connection,
    thread: ThreadId,
    at: Timestamp,
) -> Result<Option<AgentTurnId>, DomainError> {
    conn.query_row(
        "SELECT id FROM agent_turn WHERE thread_id = ?1 AND started_at <= ?2
          ORDER BY started_at DESC, id DESC LIMIT 1",
        params![thread.value(), ts_to_string(at)],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|id| id.map(AgentTurnId::new))
    .map_err(map_sql_err)
}

/// The thread's open turns, newest first.
pub fn open_turn_ids_tx(
    conn: &Connection,
    thread: ThreadId,
) -> Result<Vec<AgentTurnId>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM agent_turn WHERE thread_id = ?1 AND ended_at IS NULL
              ORDER BY started_at DESC, id DESC",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map([thread.value()], |r| r.get::<_, i64>(0))
        .map_err(map_sql_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)?;
    Ok(rows.into_iter().map(AgentTurnId::new).collect())
}

/// Open a turn on `thread` and log `agent.turn.started`, in the caller's
/// transaction. The turn starts at its stream's current snapshot.
pub fn open_turn_tx(
    conn: &Connection,
    ev: &EventCtx<'_>,
    thread: ThreadId,
    prompt: &str,
    session: Option<&str>,
    started_at: Timestamp,
) -> Result<AgentTurnId, DomainError> {
    conn.execute(
        "INSERT INTO agent_turn (thread_id, prompt, session_id, started_at) VALUES (?1, ?2, ?3, ?4)",
        params![thread.value(), prompt, session, ts_to_string(started_at)],
    )
    .map_err(map_sql_err)?;
    let id = AgentTurnId::new(conn.last_insert_rowid());
    let anchors = activity_anchors_tx(conn, thread)?;
    if let Some(stream) = anchors.stream_id {
        // Its diff runs from here to the snapshot it ends at.
        let start =
            crate::analytics_stores::current_snapshot_tx(conn, stream).map_err(map_sql_err)?;
        conn.execute(
            "UPDATE agent_turn SET start_snapshot_id = ?2 WHERE id = ?1",
            params![id.value(), start],
        )
        .map_err(map_sql_err)?;
    }
    let env = ev
        .typed::<AgentTurnStarted>(&AgentTurnStartedV1 {
            turn: turn_ref(id),
            thread: thread_ref(thread),
            session: session.map(str::to_string),
        })
        .with_anchors(Anchors {
            turn_id: Some(id.value()),
            ..anchors
        })
        .with_subject([turn_ref(id), thread_ref(thread)]);
    ev.append(conn, &env)?;
    Ok(id)
}

/// How a turn ended.
#[derive(Debug, Clone)]
pub struct TurnEnd<'a> {
    /// When: the time of the hook (or command) that ended it.
    pub at: Timestamp,
    pub outcome: TurnOutcome,
    pub answer: Option<&'a str>,
    pub transcript_path: Option<&'a str>,
    /// Counts the harness reported with the turn (ACP).
    pub usage: Option<oxplow_domain::events::schema::TurnUsage>,
}

impl TurnEnd<'_> {
    pub fn new(at: Timestamp, outcome: TurnOutcome) -> Self {
        Self {
            at,
            outcome,
            answer: None,
            transcript_path: None,
            usage: None,
        }
    }
}

/// Close turn `id` and log `agent.turn.ended`, in the caller's
/// transaction. Returns its thread, or `None` when it was already closed
/// (nothing logged).
pub fn close_turn_tx(
    conn: &Connection,
    ev: &EventCtx<'_>,
    id: AgentTurnId,
    end: &TurnEnd<'_>,
) -> Result<Option<ThreadId>, DomainError> {
    let thread: Option<i64> = conn
        .query_row(
            "UPDATE agent_turn SET ended_at = ?2, answer = COALESCE(?3, answer)
              WHERE id = ?1 AND ended_at IS NULL
              RETURNING thread_id",
            params![id.value(), ts_to_string(end.at), end.answer],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    let Some(thread) = thread.map(ThreadId::new) else {
        return Ok(None);
    };
    let env = ev
        .typed::<AgentTurnEnded>(&AgentTurnEndedV2 {
            turn: turn_ref(id),
            thread: thread_ref(thread),
            outcome: end.outcome,
            transcript_path: end.transcript_path.map(str::to_string),
            usage: end.usage.clone(),
        })
        .with_anchors(Anchors {
            turn_id: Some(id.value()),
            ..activity_anchors_tx(conn, thread)?
        })
        .with_subject([turn_ref(id), thread_ref(thread)]);
    ev.append(conn, &env)?;
    Ok(Some(thread))
}

// -- Agent status ------------------------------------------------------

/// One `agent.status.changed` row as the thread's status, read at the
/// type's newest version.
fn row_to_status(
    vocabulary: &Vocabulary,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AgentStatus> {
    let thread: i64 = row.get(0)?;
    let v: u32 = row.get(1)?;
    let payload: String = row.get(2)?;
    let at: String = row.get(3)?;
    let decode = || -> Result<AgentStatus, DomainError> {
        let value = serde_json::from_str(&payload)
            .map_err(|e| DomainError::Invalid(format!("agent.status.changed payload: {e}")))?;
        let (_, value) = vocabulary.upcast_to_latest(AgentStatusChanged::TYPE, v, value)?;
        let p: <AgentStatusChanged as EventType>::Payload = serde_json::from_value(value)
            .map_err(|e| DomainError::Invalid(format!("agent.status.changed payload: {e}")))?;
        Ok(AgentStatus {
            thread_id: ThreadId::new(thread),
            state: p.state.into(),
            detail: p.detail,
            updated_at: string_to_ts(&at)?,
        })
    };
    decode().map_err(map_err_text)
}

/// The thread's current status: its newest `agent.status.changed`.
pub fn last_status_tx(
    conn: &Connection,
    vocabulary: &Vocabulary,
    thread: ThreadId,
) -> Result<Option<AgentStatus>, DomainError> {
    conn.query_row(
        "SELECT thread_id, v, payload, at FROM event_log
          WHERE thread_id = ?1 AND type = ?2
          ORDER BY seq DESC LIMIT 1",
        params![thread.value(), AgentStatusChanged::TYPE],
        |r| row_to_status(vocabulary, r),
    )
    .optional()
    .map_err(map_sql_err)
}

/// Every existing thread's current status.
fn all_statuses_tx(
    conn: &Connection,
    vocabulary: &Vocabulary,
) -> Result<Vec<AgentStatus>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT e.thread_id, e.v, e.payload, e.at
               FROM event_log e JOIN threads th ON th.id = e.thread_id
              WHERE e.seq IN (SELECT MAX(seq) FROM event_log
                               WHERE type = ?1 AND thread_id IS NOT NULL
                               GROUP BY thread_id)
              ORDER BY e.thread_id",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map([AgentStatusChanged::TYPE], |r| row_to_status(vocabulary, r))
        .map_err(map_sql_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)?;
    Ok(rows)
}

/// Agent status read from the event log. There is no write side: the
/// hook ingest logs `agent.status.changed` and that is the record.
#[derive(Clone)]
pub struct SqliteAgentStatusStore {
    db: Database,
    vocabulary: VocabularyHandle,
}

impl SqliteAgentStatusStore {
    pub fn new(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self { db, vocabulary }
    }
}

#[async_trait]
impl AgentStatusStore for SqliteAgentStatusStore {
    async fn get(&self, thread: &ThreadId) -> Result<Option<AgentStatus>, DomainError> {
        let (vocabulary, thread) = (self.vocabulary.clone(), *thread);
        self.db
            .call_mut(move |c| last_status_tx(c, &vocabulary.current(), thread))
            .await
    }

    async fn list_all(&self) -> Result<Vec<AgentStatus>, DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .call_mut(move |c| all_statuses_tx(c, &vocabulary.current()))
            .await
    }
}

/// `agent_turn` rows, and the `agent.turn.started` / `agent.turn.ended`
/// events logged in the same transaction as the row changes (P2.3).
#[derive(Clone)]
pub struct SqliteAgentTurnStore {
    db: Database,
    vocabulary: VocabularyHandle,
}

impl SqliteAgentTurnStore {
    pub fn new(db: Database) -> Self {
        Self::with_vocabulary(db, VocabularyHandle::core())
    }

    pub fn with_vocabulary(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self { db, vocabulary }
    }

    /// Whether any thread of `stream` has a turn open — the quiet-period
    /// trigger's "is an agent working here?" probe.
    pub async fn stream_has_open_turn(&self, stream: StreamId) -> Result<bool, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM agent_turn t JOIN threads th ON th.id = t.thread_id
                                     WHERE th.stream_id = ?1 AND t.ended_at IS NULL)",
                    params![stream.value()],
                    |r| r.get(0),
                )
            })
            .await
    }
}

#[async_trait]
impl AgentTurnStore for SqliteAgentTurnStore {
    async fn open(&self, turn: &AgentTurn) -> Result<AgentTurnId, DomainError> {
        let turn = turn.clone();
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                if turn.id.is_placeholder() {
                    let vocabulary = vocabulary.current();
                    let ev = EventCtx::system(&vocabulary, "hook_ingest");
                    return open_turn_tx(
                        tx,
                        &ev,
                        turn.thread_id,
                        &turn.prompt,
                        turn.session_id.as_deref(),
                        turn.started_at,
                    );
                }
                // Re-writing an existing id is an update, not a new turn.
                tx.execute(
                    "INSERT INTO agent_turn (id, thread_id, prompt, answer, session_id, started_at, ended_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                        prompt = excluded.prompt,
                        session_id = excluded.session_id",
                    params![
                        turn.id.value(),
                        turn.thread_id.value(),
                        turn.prompt,
                        turn.answer,
                        turn.session_id,
                        ts_to_string(turn.started_at),
                        turn.ended_at.map(ts_to_string),
                    ],
                )
                .map_err(map_sql_err)?;
                Ok(turn.id)
            })
            .await
    }

    /// Outside the hook ingest (which closes in its own transaction), a
    /// turn is closed by restart recovery: `Restart` logs as
    /// `system:recovery`.
    async fn close(
        &self,
        id: &AgentTurnId,
        answer: Option<String>,
        outcome: TurnOutcome,
    ) -> Result<bool, DomainError> {
        let id = *id;
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let component = match outcome {
                    TurnOutcome::Restart => "recovery",
                    _ => "hook_ingest",
                };
                let vocabulary = vocabulary.current();
                let ev = EventCtx::system(&vocabulary, component);
                let end = TurnEnd {
                    answer: answer.as_deref(),
                    ..TurnEnd::new(Timestamp::now(), outcome)
                };
                Ok(close_turn_tx(tx, &ev, id, &end)?.is_some())
            })
            .await
    }

    async fn get(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM agent_turn WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], row_to_turn)?;
                rows.next().transpose()
            })
            .await
    }

    async fn list_open(&self, thread: &ThreadId) -> Result<Vec<AgentTurn>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![thread.value()], row_to_turn)?;
                rows.collect()
            })
            .await
    }

    async fn list_all_open(&self) -> Result<Vec<AgentTurn>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE ended_at IS NULL
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map([], row_to_turn)?;
                rows.collect()
            })
            .await
    }

    async fn list_for_thread(
        &self,
        thread: &ThreadId,
        limit: usize,
    ) -> Result<Vec<AgentTurn>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1
                     ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![thread.value(), limit as i64], row_to_turn)?;
                rows.collect()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    async fn fixture() -> (Database, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let now = Timestamp::from_unix_ms(1);
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (db, t.id)
    }

    fn log_status(
        db: &Database,
        vocabulary: &Vocabulary,
        thread: ThreadId,
        state: oxplow_domain::AgentStatusState,
        detail: Option<&str>,
    ) {
        use oxplow_domain::events::schema::AgentStatusChangedV1;
        let conn = db.conn().unwrap();
        let ev = EventCtx::system(vocabulary, "test");
        let env = ev
            .typed::<AgentStatusChanged>(&AgentStatusChangedV1 {
                thread: thread_ref(thread),
                state: oxplow_domain::events::schema::LoggedAgentStatus::of(state).unwrap(),
                detail: detail.map(str::to_string),
            })
            .with_anchors(anchors_for_thread_tx(&conn, thread).unwrap())
            .with_subject([thread_ref(thread)]);
        ev.append(&conn, &env).unwrap();
    }

    #[tokio::test]
    async fn status_is_the_newest_logged_status_change() {
        use oxplow_domain::AgentStatusState as S;
        let (db, tid) = fixture().await;
        let vocabulary = VocabularyHandle::core();
        let store = SqliteAgentStatusStore::new(db.clone(), vocabulary.clone());
        assert!(store.get(&tid).await.unwrap().is_none());
        assert!(store.list_all().await.unwrap().is_empty());

        log_status(&db, &vocabulary.current(), tid, S::Running, None);
        log_status(
            &db,
            &vocabulary.current(),
            tid,
            S::AwaitingUser,
            Some("A or B?"),
        );

        // A fresh store over the same database (a restarted daemon) reads it.
        let store = SqliteAgentStatusStore::new(db.clone(), vocabulary);
        let got = store.get(&tid).await.unwrap().unwrap();
        assert_eq!(got.state, S::AwaitingUser);
        assert_eq!(got.detail.as_deref(), Some("A or B?"));
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], got);
    }

    #[tokio::test]
    async fn agent_turn_open_then_close() {
        let (db, tid) = fixture().await;
        let store = SqliteAgentTurnStore::new(db.clone());
        let turn = AgentTurn {
            id: AgentTurnId::placeholder(),
            thread_id: tid,
            prompt: "do the thing".into(),
            answer: None,
            session_id: Some("s1".into()),
            started_at: Timestamp::now(),
            ended_at: None,
            start_snapshot_id: None,
            snapshot_id: None,
        };
        let id = store.open(&turn).await.unwrap();
        let open = store.list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        store
            .close(&id, Some("done".into()), TurnOutcome::Completed)
            .await
            .unwrap();
        // A second close is a no-op: no second event.
        store
            .close(&id, None, TurnOutcome::Interrupted)
            .await
            .unwrap();
        let still_open = store.list_open(&tid).await.unwrap();
        assert!(still_open.is_empty());
        let got = store.get(&id).await.unwrap().unwrap();
        assert!(got.ended_at.is_some());
        assert_eq!(got.answer.as_deref(), Some("done"));

        let stream = StreamId::new(r_stream(&db, tid));
        assert!(!store.stream_has_open_turn(stream).await.unwrap());

        // Both edges are in the log, anchored to the turn, thread and stream.
        let conn = db.conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT type, payload, turn_id, thread_id, stream_id FROM event_log ORDER BY seq",
            )
            .unwrap();
        struct Logged {
            ty: String,
            payload: serde_json::Value,
            anchors: [Option<i64>; 3],
        }
        let rows: Vec<Logged> = stmt
            .query_map([], |r| {
                Ok(Logged {
                    ty: r.get(0)?,
                    payload: serde_json::from_str(&r.get::<_, String>(1)?).unwrap(),
                    anchors: [r.get(2)?, r.get(3)?, r.get(4)?],
                })
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ty, "agent.turn.started");
        assert_eq!(rows[0].payload["turn"], format!("turn:{id}"));
        assert_eq!(rows[0].payload["session"], "s1");
        assert_eq!(rows[1].ty, "agent.turn.ended");
        assert_eq!(rows[1].payload["outcome"], "completed");
        for r in &rows {
            let [turn, thread, stream_anchor] = r.anchors;
            assert_eq!(turn, Some(id.value()));
            assert_eq!(thread, Some(tid.value()));
            assert_eq!(stream_anchor, Some(stream.value()));
        }
    }

    fn r_stream(db: &Database, t: ThreadId) -> i64 {
        db.conn()
            .unwrap()
            .query_row(
                "SELECT stream_id FROM threads WHERE id = ?1",
                params![t.value()],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// P3.3 (tsk473): outside the hook ingest a turn is closed by restart
    /// recovery, and its `agent.turn.ended` says so.
    #[tokio::test]
    async fn a_restart_close_is_logged_as_recovery() {
        let db = Database::in_memory();
        let now = "2026-09-29T00:00:00.000000Z";
        let seed = format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{now}', '{now}');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (1, 1, 't', 'active', '{now}', '{now}');"
        );
        db.transaction(move |tx| tx.execute_batch(&seed).map_err(map_sql_err))
            .await
            .unwrap();
        let store = SqliteAgentTurnStore::new(db.clone());
        let id = store
            .open(&AgentTurn {
                id: AgentTurnId::placeholder(),
                thread_id: ThreadId::new(1),
                prompt: "p".into(),
                answer: None,
                session_id: None,
                started_at: Timestamp::now(),
                ended_at: None,
                start_snapshot_id: None,
                snapshot_id: None,
            })
            .await
            .unwrap();
        assert!(store.close(&id, None, TurnOutcome::Restart).await.unwrap());
        let source: String = db
            .transaction(|tx| {
                tx.query_row(
                    "SELECT source FROM event_log WHERE type = 'agent.turn.ended'",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(source, "system:recovery");
    }
}
