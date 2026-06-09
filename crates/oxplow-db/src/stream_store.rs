use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{DomainError, Stream, StreamId, StreamKind, Timestamp};

use crate::database::Database;

#[derive(Clone)]
pub struct SqliteStreamStore {
    db: Database,
}

impl SqliteStreamStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn kind_to_str(k: StreamKind) -> &'static str {
    match k {
        StreamKind::Primary => "primary",
        StreamKind::Worktree => "worktree",
    }
}

fn str_to_kind(s: &str) -> Result<StreamKind, DomainError> {
    match s {
        "primary" => Ok(StreamKind::Primary),
        "worktree" => Ok(StreamKind::Worktree),
        other => Err(DomainError::Invalid(format!(
            "unknown stream kind: {other}"
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

fn row_to_stream(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stream> {
    let id: i64 = row.get("id")?;
    let kind: String = row.get("kind")?;
    let title: String = row.get("title")?;
    let branch: String = row.get("branch")?;
    let branch_ref: String = row.get("branch_ref")?;
    let branch_source: String = row.get("branch_source")?;
    let worktree_path: String = row.get("worktree_path")?;
    let working_pane: String = row.get("working_pane")?;
    let talking_pane: String = row.get("talking_pane")?;
    let working_session_id: String = row.get("working_session_id")?;
    let talking_session_id: String = row.get("talking_session_id")?;
    let custom_prompt: Option<String> = row.get("custom_prompt")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let archived_at: Option<String> = row.get("archived_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(Stream {
        id: StreamId::new(id),
        kind: str_to_kind(&kind).map_err(map_err)?,
        title,
        branch,
        branch_ref,
        branch_source,
        worktree_path,
        working_pane,
        talking_pane,
        working_session_id,
        talking_session_id,
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
impl StreamStore for SqliteStreamStore {
    async fn list(&self) -> Result<Vec<Stream>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM streams \
                     WHERE archived_at IS NULL \
                     ORDER BY \
                     CASE kind WHEN 'primary' THEN 0 ELSE 1 END, created_at ASC",
                )?;
                let rows = stmt.query_map([], row_to_stream)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn get(&self, id: &StreamId) -> Result<Option<Stream>, DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM streams WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.value()], row_to_stream)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn upsert(&self, stream: &Stream) -> Result<StreamId, DomainError> {
        let stream = stream.clone();
        self.db
            .call(move |conn| {
                // Placeholder id → NULL so SQLite autoincrements; an
                // explicit id is inserted / upserted in place.
                let id_param: Option<i64> = if stream.id.is_placeholder() {
                    None
                } else {
                    Some(stream.id.value())
                };
                conn.execute(
                    "INSERT INTO streams (
                        id, kind, title, branch, branch_ref, branch_source,
                        worktree_path, working_pane, talking_pane,
                        working_session_id, talking_session_id, custom_prompt,
                        created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                     ON CONFLICT(id) DO UPDATE SET
                        kind = excluded.kind,
                        title = excluded.title,
                        branch = excluded.branch,
                        branch_ref = excluded.branch_ref,
                        branch_source = excluded.branch_source,
                        worktree_path = excluded.worktree_path,
                        working_pane = excluded.working_pane,
                        talking_pane = excluded.talking_pane,
                        working_session_id = excluded.working_session_id,
                        talking_session_id = excluded.talking_session_id,
                        custom_prompt = excluded.custom_prompt,
                        updated_at = excluded.updated_at",
                    params![
                        id_param,
                        kind_to_str(stream.kind),
                        stream.title,
                        stream.branch,
                        stream.branch_ref,
                        stream.branch_source,
                        stream.worktree_path,
                        stream.working_pane,
                        stream.talking_pane,
                        stream.working_session_id,
                        stream.talking_session_id,
                        stream.custom_prompt,
                        ts_to_string(stream.created_at),
                        ts_to_string(stream.updated_at),
                    ],
                )?;
                Ok(if stream.id.is_placeholder() {
                    StreamId::new(conn.last_insert_rowid())
                } else {
                    stream.id
                })
            })
            .await
    }

    async fn delete(&self, id: &StreamId) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM streams WHERE id = ?1", params![id.value()])?;
                Ok(())
            })
            .await
    }

    async fn archive(&self, id: &StreamId) -> Result<(), DomainError> {
        let id = *id;
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "UPDATE streams SET archived_at = COALESCE(archived_at, ?2),
                                          updated_at = ?2
                     WHERE id = ?1",
                    params![id.value(), now],
                )?;
                Ok(())
            })
            .await
    }

    async fn current_id(&self) -> Result<Option<StreamId>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT current_stream_id FROM runtime_state WHERE id = 1")?;
                let mut rows = stmt.query_map([], |r| r.get::<_, Option<i64>>(0))?;
                match rows.next() {
                    Some(Ok(Some(s))) => Ok(Some(StreamId::new(s))),
                    Some(Ok(None)) => Ok(None),
                    Some(Err(e)) => Err(e),
                    None => Ok(None),
                }
            })
            .await
    }

    async fn set_current(&self, id: Option<&StreamId>) -> Result<(), DomainError> {
        let id = id.cloned();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE runtime_state SET current_stream_id = ?1 WHERE id = 1",
                    params![id.as_ref().map(|s| s.value())],
                )?;
                Ok(())
            })
            .await
    }

    async fn primary(&self) -> Result<Option<Stream>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT * FROM streams WHERE kind = 'primary' LIMIT 1")?;
                let mut rows = stmt.query_map([], row_to_stream)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn primary() -> Stream {
        Stream {
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
        }
    }

    #[tokio::test]
    async fn upsert_then_get() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let s = primary();
        store.upsert(&s).await.unwrap();
        let got = store.get(&s.id).await.unwrap().unwrap();
        assert_eq!(got, s);
    }

    #[tokio::test]
    async fn list_orders_primary_first() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let p = primary();
        let mut wt = primary();
        wt.id = StreamId::new(2);
        wt.kind = StreamKind::Worktree;
        wt.created_at = Timestamp::from_unix_ms(1_700_000_001_000);
        wt.updated_at = wt.created_at;
        store.upsert(&wt).await.unwrap();
        store.upsert(&p).await.unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].kind, StreamKind::Primary);
        assert_eq!(list[1].kind, StreamKind::Worktree);
    }

    #[tokio::test]
    async fn primary_invariant_unique() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let p = primary();
        store.upsert(&p).await.unwrap();
        let mut p2 = primary();
        p2.id = StreamId::new(3);
        let err = store.upsert(&p2).await.unwrap_err();
        // Unique index on kind='primary' enforces single-primary invariant.
        assert!(matches!(err, DomainError::Invalid(_)));
    }

    #[tokio::test]
    async fn primary_returns_the_primary() {
        let store = SqliteStreamStore::new(Database::in_memory());
        store.upsert(&primary()).await.unwrap();
        let got = store.primary().await.unwrap().unwrap();
        assert_eq!(got.kind, StreamKind::Primary);
    }

    #[tokio::test]
    async fn current_stream_pointer_round_trips() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let s = primary();
        store.upsert(&s).await.unwrap();
        assert_eq!(store.current_id().await.unwrap(), None);
        store.set_current(Some(&s.id)).await.unwrap();
        assert_eq!(store.current_id().await.unwrap(), Some(s.id));
        store.set_current(None).await.unwrap();
        assert_eq!(store.current_id().await.unwrap(), None);
    }

    #[tokio::test]
    async fn deleting_stream_clears_current_pointer() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let s = primary();
        store.upsert(&s).await.unwrap();
        store.set_current(Some(&s.id)).await.unwrap();
        store.delete(&s.id).await.unwrap();
        assert_eq!(store.current_id().await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let store = SqliteStreamStore::new(Database::in_memory());
        let s = primary();
        store.upsert(&s).await.unwrap();
        store.delete(&s.id).await.unwrap();
        assert!(store.get(&s.id).await.unwrap().is_none());
    }
}
