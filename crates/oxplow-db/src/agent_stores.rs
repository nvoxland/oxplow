//! Sqlite-backed `AgentTurnStore`. (hook_event and agent_status used
//! to live here too; they're now in-memory in
//! `oxplow_app::thread_runtime::ThreadRuntimeRegistry` since the data
//! is per-instance transient and was being reset on every boot
//! anyway.)

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::{AgentTurn, AgentTurnId, DomainError, TaskId, ThreadId, Timestamp};

use crate::database::Database;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn map_err_text(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

// -- AgentTurn ---------------------------------------------------------

fn row_to_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTurn> {
    let id: String = row.get("id")?;
    let thread_id: String = row.get("thread_id")?;
    let task_id: Option<i64> = row.get("task_id")?;
    let prompt: String = row.get("prompt")?;
    let answer: Option<String> = row.get("answer")?;
    let session_id: Option<String> = row.get("session_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    Ok(AgentTurn {
        id: AgentTurnId::from(id),
        thread_id: ThreadId::from(thread_id),
        task_id: task_id.map(TaskId::new),
        prompt,
        answer,
        session_id,
        started_at: string_to_ts(&started_at).map_err(map_err_text)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err_text)?,
    })
}

#[derive(Clone)]
pub struct SqliteAgentTurnStore {
    db: Database,
}

impl SqliteAgentTurnStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AgentTurnStore for SqliteAgentTurnStore {
    async fn open(&self, turn: &AgentTurn) -> Result<(), DomainError> {
        let turn = turn.clone();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO agent_turn (id, thread_id, task_id, prompt, answer, session_id, started_at, ended_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                        prompt = excluded.prompt,
                        task_id = excluded.task_id,
                        session_id = excluded.session_id",
                    params![
                        turn.id.as_str(),
                        turn.thread_id.as_str(),
                        turn.task_id.map(|w| w.value()),
                        turn.prompt,
                        turn.answer,
                        turn.session_id,
                        ts_to_string(turn.started_at),
                        turn.ended_at.map(ts_to_string),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    async fn close(&self, id: &AgentTurnId, answer: Option<String>) -> Result<(), DomainError> {
        let id = id.clone();
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE agent_turn SET ended_at = ?2, answer = COALESCE(?3, answer)
                     WHERE id = ?1 AND ended_at IS NULL",
                    params![id.as_str(), now, answer],
                )?;
                Ok(())
            })
            .await
    }

    async fn get(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, DomainError> {
        let id = id.clone();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM agent_turn WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.as_str()], row_to_turn)?;
                rows.next().transpose()
            })
            .await
    }

    async fn list_open(&self, thread: &ThreadId) -> Result<Vec<AgentTurn>, DomainError> {
        let thread = thread.clone();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![thread.as_str()], row_to_turn)?;
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
        let thread = thread.clone();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1
                     ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![thread.as_str(), limit as i64], row_to_turn)?;
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
            id: StreamId::from("s-1"),
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
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
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
        let store = SqliteAgentTurnStore::new(db);
        let turn = AgentTurn {
            id: AgentTurnId::new(),
            thread_id: tid.clone(),
            task_id: None,
            prompt: "do the thing".into(),
            answer: None,
            session_id: None,
            started_at: Timestamp::now(),
            ended_at: None,
        };
        store.open(&turn).await.unwrap();
        let open = store.list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        store.close(&turn.id, Some("done".into())).await.unwrap();
        let still_open = store.list_open(&tid).await.unwrap();
        assert!(still_open.is_empty());
        let got = store.get(&turn.id).await.unwrap().unwrap();
        assert!(got.ended_at.is_some());
        assert_eq!(got.answer.as_deref(), Some("done"));
    }
}
