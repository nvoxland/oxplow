use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, DomainError, StreamId, Thread, ThreadId, ThreadStatus, Timestamp};

use crate::database::Database;

#[derive(Clone)]
pub struct SqliteThreadStore {
    db: Database,
}

impl SqliteThreadStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn status_to_str(s: ThreadStatus) -> &'static str {
    match s {
        ThreadStatus::Active => "active",
        ThreadStatus::Queued => "queued",
        ThreadStatus::Closed => "closed",
    }
}

fn str_to_status(s: &str) -> Result<ThreadStatus, DomainError> {
    match s {
        "active" => Ok(ThreadStatus::Active),
        "queued" => Ok(ThreadStatus::Queued),
        "closed" => Ok(ThreadStatus::Closed),
        other => Err(DomainError::Invalid(format!(
            "unknown thread status: {other}"
        ))),
    }
}

fn agent_to_str(agent: AgentKind) -> &'static str {
    agent.as_str()
}

fn str_to_agent(s: &str) -> Result<AgentKind, DomainError> {
    match s {
        "claude" => Ok(AgentKind::Claude),
        "codex" => Ok(AgentKind::Codex),
        "opencode" => Ok(AgentKind::Opencode),
        other => Err(DomainError::Invalid(format!(
            "unknown thread agent: {other}"
        ))),
    }
}

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

fn row_to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thread> {
    let id: i64 = row.get("id")?;
    let stream_id: i64 = row.get("stream_id")?;
    let title: String = row.get("title")?;
    let status: String = row.get("status")?;
    let sort_index: i64 = row.get("sort_index")?;
    let pane_target: String = row.get("pane_target")?;
    let agent: String = row.get("agent")?;
    let resume_session_id: String = row.get("resume_session_id")?;
    let summary: String = row.get("summary")?;
    let summary_updated_at: Option<String> = row.get("summary_updated_at")?;
    let closed_at: Option<String> = row.get("closed_at")?;
    let archived_at: Option<String> = row.get("archived_at")?;
    let custom_prompt: Option<String> = row.get("custom_prompt")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Thread {
        id: ThreadId::new(id),
        stream_id: StreamId::new(stream_id),
        title,
        status: str_to_status(&status).map_err(map_err)?,
        sort_index,
        pane_target,
        agent: str_to_agent(&agent).map_err(map_err)?,
        resume_session_id,
        summary,
        summary_updated_at: summary_updated_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        closed_at: closed_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        custom_prompt,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
        archived_at: archived_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
    })
}

#[async_trait]
impl ThreadStore for SqliteThreadStore {
    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<Thread>, DomainError> {
        let stream = *stream;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM threads \
                     WHERE stream_id = ?1 AND archived_at IS NULL \
                     ORDER BY sort_index ASC, created_at ASC",
                )?;
                let rows = stmt.query_map(params![stream.value()], row_to_thread)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM threads WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], row_to_thread)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn upsert(&self, thread: &Thread) -> Result<ThreadId, DomainError> {
        let thread = thread.clone();
        self.db
            .call(move |conn| {
                let id_param: Option<i64> = if thread.id.is_placeholder() {
                    None
                } else {
                    Some(thread.id.value())
                };
                conn.execute(
                    "INSERT INTO threads (
                        id, stream_id, title, status, sort_index, pane_target, agent,
                        resume_session_id, summary, summary_updated_at, closed_at,
                        custom_prompt, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                     ON CONFLICT(id) DO UPDATE SET
                        title = excluded.title,
                        status = excluded.status,
                        sort_index = excluded.sort_index,
                        pane_target = excluded.pane_target,
                        agent = excluded.agent,
                        resume_session_id = excluded.resume_session_id,
                        summary = excluded.summary,
                        summary_updated_at = excluded.summary_updated_at,
                        closed_at = excluded.closed_at,
                        custom_prompt = excluded.custom_prompt,
                        updated_at = excluded.updated_at",
                    params![
                        id_param,
                        thread.stream_id.value(),
                        thread.title,
                        status_to_str(thread.status),
                        thread.sort_index,
                        thread.pane_target,
                        agent_to_str(thread.agent),
                        thread.resume_session_id,
                        thread.summary,
                        thread.summary_updated_at.map(ts_to_string),
                        thread.closed_at.map(ts_to_string),
                        thread.custom_prompt,
                        ts_to_string(thread.created_at),
                        ts_to_string(thread.updated_at),
                    ],
                )?;
                Ok(if thread.id.is_placeholder() {
                    ThreadId::new(conn.last_insert_rowid())
                } else {
                    thread.id
                })
            })
            .await
    }

    async fn delete(&self, id: &ThreadId) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM threads WHERE id = ?1", params![id.value()])?;
                Ok(())
            })
            .await
    }

    async fn archive(&self, id: &ThreadId) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "UPDATE threads SET archived_at = COALESCE(archived_at, ?2),
                                          updated_at = ?2
                     WHERE id = ?1",
                    params![id.value(), now],
                )?;
                Ok(())
            })
            .await
    }

    async fn selected_for_stream(
        &self,
        stream: &StreamId,
    ) -> Result<Option<ThreadId>, DomainError> {
        let stream = *stream;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT selected_thread_id FROM thread_selection WHERE stream_id = ?1",
                )?;
                let mut rows =
                    stmt.query_map(params![stream.value()], |r| r.get::<_, Option<i64>>(0))?;
                match rows.next() {
                    Some(Ok(Some(s))) => Ok(Some(ThreadId::new(s))),
                    Some(Ok(None)) => Ok(None),
                    Some(Err(e)) => Err(e),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn set_selected_for_stream(
        &self,
        stream: &StreamId,
        thread: Option<&ThreadId>,
    ) -> Result<(), DomainError> {
        let stream = *stream;
        let thread = thread.cloned();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO thread_selection (stream_id, selected_thread_id)
                     VALUES (?1, ?2)
                     ON CONFLICT(stream_id) DO UPDATE SET selected_thread_id = excluded.selected_thread_id",
                    params![stream.value(), thread.as_ref().map(|t| t.value())],
                )?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{Stream, StreamKind};

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn make_store() -> (SqliteThreadStore, StreamId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        (SqliteThreadStore::new(db), s.id)
    }

    fn thread(stream_id: StreamId) -> Thread {
        Thread {
            id: ThreadId::new(1),
            stream_id,
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        }
    }

    #[tokio::test]
    async fn upsert_then_get() {
        let (store, sid) = make_store().await;
        let t = thread(sid);
        store.upsert(&t).await.unwrap();
        assert_eq!(store.get(&t.id).await.unwrap().unwrap(), t);
    }

    #[tokio::test]
    async fn every_agent_kind_round_trips() {
        let (store, sid) = make_store().await;
        for (i, agent) in [
            oxplow_domain::AgentKind::Claude,
            oxplow_domain::AgentKind::Codex,
            oxplow_domain::AgentKind::Opencode,
        ]
        .into_iter()
        .enumerate()
        {
            let mut t = thread(sid);
            t.id = ThreadId::new(100 + i as i64);
            t.status = ThreadStatus::Queued;
            t.agent = agent;
            store.upsert(&t).await.unwrap();
            assert_eq!(store.get(&t.id).await.unwrap().unwrap().agent, agent);
        }
    }

    #[tokio::test]
    async fn upsert_overwrites_existing() {
        let (store, sid) = make_store().await;
        let mut t = thread(sid);
        store.upsert(&t).await.unwrap();
        t.title = "updated".into();
        t.status = ThreadStatus::Closed;
        store.upsert(&t).await.unwrap();
        let got = store.get(&t.id).await.unwrap().unwrap();
        assert_eq!(got.title, "updated");
        assert_eq!(got.status, ThreadStatus::Closed);
    }

    #[tokio::test]
    async fn at_most_one_active_thread_per_stream() {
        let (store, sid) = make_store().await;
        let mut a = thread(sid);
        a.id = ThreadId::new(10);
        a.status = ThreadStatus::Active;
        let mut b = thread(sid);
        b.id = ThreadId::new(11);
        b.status = ThreadStatus::Active;
        store.upsert(&a).await.unwrap();
        // Second active thread on the same stream violates the
        // partial unique index → DB error → DomainError::Constraint.
        let err = store.upsert(&b).await.unwrap_err();
        assert!(matches!(err, DomainError::Constraint(_)));
    }

    #[tokio::test]
    async fn list_for_stream_orders_by_sort_index() {
        let (store, sid) = make_store().await;
        // Only one Active per stream — give the second thread Queued
        // status so we don't trip the partial unique index.
        let mut a = thread(sid);
        a.id = ThreadId::new(10);
        a.sort_index = 1;
        a.status = ThreadStatus::Active;
        let mut b = thread(sid);
        b.id = ThreadId::new(11);
        b.sort_index = 0;
        b.status = ThreadStatus::Queued;
        store.upsert(&a).await.unwrap();
        store.upsert(&b).await.unwrap();
        let list = store.list_for_stream(&sid).await.unwrap();
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    #[tokio::test]
    async fn selected_thread_round_trips_per_stream() {
        let (store, sid) = make_store().await;
        let t = thread(sid);
        store.upsert(&t).await.unwrap();
        assert_eq!(store.selected_for_stream(&sid).await.unwrap(), None);
        store
            .set_selected_for_stream(&sid, Some(&t.id))
            .await
            .unwrap();
        assert_eq!(store.selected_for_stream(&sid).await.unwrap(), Some(t.id));
        store.set_selected_for_stream(&sid, None).await.unwrap();
        assert_eq!(store.selected_for_stream(&sid).await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_cascades_when_stream_dropped() {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db);
        let s = Stream {
            id: StreamId::new(2),
            kind: StreamKind::Worktree,
            title: "wt".into(),
            branch: "feat".into(),
            branch_ref: "refs/heads/feat".into(),
            branch_source: "main".into(),
            worktree_path: "/repo/wt".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = thread(s.id);
        threads.upsert(&t).await.unwrap();
        streams.delete(&s.id).await.unwrap();
        assert!(threads.get(&t.id).await.unwrap().is_none());
    }
}
