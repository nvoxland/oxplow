//! Sqlite-backed `AgentTurnStore`. (hook_event and agent_status used
//! to live here too; they're now in-memory in
//! `oxplow_app::thread_runtime::ThreadRuntimeRegistry` since the data
//! is per-instance transient and was being reset on every boot
//! anyway.)

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};

use oxplow_domain::events::schema::{
    AgentTurnEnded, AgentTurnEndedV2, AgentTurnStarted, AgentTurnStartedV1, EventSchemaRegistry,
};
use oxplow_domain::events::{Anchors, Envelope};
use oxplow_domain::hook::TurnOutcome;
use oxplow_domain::refs::build::{system_source, thread_ref, turn_ref};
use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::{AgentTurn, AgentTurnId, DomainError, StreamId, ThreadId, Timestamp};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::append_tx;

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

/// The anchors a turn's events carry: its thread and that thread's stream.
fn turn_anchors(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    turn: AgentTurnId,
) -> Result<Anchors, DomainError> {
    let stream: Option<i64> = conn
        .query_row(
            "SELECT stream_id FROM threads WHERE id = ?1",
            params![thread.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    Ok(Anchors {
        stream_id: stream.map(StreamId::new),
        thread_id: Some(thread),
        turn_id: Some(turn.value()),
        ..Anchors::default()
    })
}

/// `agent_turn` rows, and the `agent.turn.started` / `agent.turn.ended`
/// events logged in the same transaction as the row changes (P2.3).
#[derive(Clone)]
pub struct SqliteAgentTurnStore {
    db: Database,
    event_schemas: Arc<EventSchemaRegistry>,
}

impl SqliteAgentTurnStore {
    pub fn new(db: Database) -> Self {
        Self::with_event_schemas(db, Arc::new(EventSchemaRegistry::core()))
    }

    pub fn with_event_schemas(db: Database, event_schemas: Arc<EventSchemaRegistry>) -> Self {
        Self { db, event_schemas }
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
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let fresh = turn.id.is_placeholder();
                let id_param: Option<i64> = (!fresh).then(|| turn.id.value());
                tx.execute(
                    "INSERT INTO agent_turn (id, thread_id, prompt, answer, session_id, started_at, ended_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                        prompt = excluded.prompt,
                        session_id = excluded.session_id",
                    params![
                        id_param,
                        turn.thread_id.value(),
                        turn.prompt,
                        turn.answer,
                        turn.session_id,
                        ts_to_string(turn.started_at),
                        turn.ended_at.map(ts_to_string),
                    ],
                )
                .map_err(map_sql_err)?;
                let id = if fresh {
                    AgentTurnId::new(tx.last_insert_rowid())
                } else {
                    turn.id
                };
                let anchors = turn_anchors(tx, turn.thread_id, id)?;
                if fresh {
                    // The turn starts at the stream's current snapshot; its
                    // diff runs from here to the snapshot it ends at.
                    if let Some(stream) = anchors.stream_id {
                        let start = crate::analytics_stores::current_snapshot_tx(tx, stream)
                            .map_err(map_sql_err)?;
                        tx.execute(
                            "UPDATE agent_turn SET start_snapshot_id = ?2 WHERE id = ?1",
                            params![id.value(), start],
                        )
                        .map_err(map_sql_err)?;
                    }
                }
                // A re-open of an existing id is an update, not a new turn.
                if fresh {
                    let env = Envelope::typed::<AgentTurnStarted>(
                        system_source("hook_ingest"),
                        &AgentTurnStartedV1 {
                            turn: turn_ref(id),
                            thread: thread_ref(turn.thread_id),
                            session: turn.session_id.clone(),
                        },
                    )
                    .with_anchors(anchors)
                    .with_subject([turn_ref(id), thread_ref(turn.thread_id)]);
                    append_tx(tx, &schemas, &env)?;
                }
                Ok(id)
            })
            .await
    }

    async fn close(
        &self,
        id: &AgentTurnId,
        answer: Option<String>,
        outcome: TurnOutcome,
    ) -> Result<bool, DomainError> {
        let id = *id;
        let now = ts_to_string(Timestamp::now());
        let schemas = self.event_schemas.clone();
        self.db
            .transaction(move |tx| {
                let thread: Option<i64> = tx
                    .query_row(
                        "UPDATE agent_turn SET ended_at = ?2, answer = COALESCE(?3, answer)
                          WHERE id = ?1 AND ended_at IS NULL
                          RETURNING thread_id",
                        params![id.value(), now, answer],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(map_sql_err)?;
                let Some(thread) = thread.map(ThreadId::new) else {
                    return Ok(false); // already closed
                };
                let env = Envelope::typed::<AgentTurnEnded>(
                    system_source("hook_ingest"),
                    &AgentTurnEndedV2 {
                        turn: turn_ref(id),
                        thread: thread_ref(thread),
                        outcome,
                        transcript_path: None,
                        usage: None,
                    },
                )
                .with_anchors(turn_anchors(tx, thread, id)?)
                .with_subject([turn_ref(id), thread_ref(thread)]);
                append_tx(tx, &schemas, &env)?;
                Ok(true)
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
}
