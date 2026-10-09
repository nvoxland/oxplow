//! Persisted agent tool calls (`agent_tool_call`), recorded from the
//! PostToolUse hook. Read through `v_tool_call`, `v_context_read` and
//! `v_struggle`. See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewToolCall {
    pub thread_id: i64,
    pub effort_id: Option<i64>,
    /// The turn it ran in (`agent_turn.id`).
    pub turn_id: Option<i64>,
    /// The harness's own name for the tool.
    pub tool: String,
    /// What it does, in oxplow's vocabulary (`ToolKind::as_str`).
    pub kind: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ok: Option<bool>,
    /// The `agent.tool.finished` event this row projects; a second row for
    /// the same event is not written.
    pub event_id: Option<String>,
    /// When it ran; now when absent.
    pub at: Option<oxplow_domain::Timestamp>,
    /// The agent session that made it (`agent_session.id`).
    pub agent_session_id: Option<i64>,
    /// The subagent it ran inside, when one is known.
    pub subagent_id: Option<String>,
    pub subagent_kind: Option<String>,
}

#[derive(Clone)]
pub struct SqliteToolCallStore {
    db: Database,
}

impl SqliteToolCallStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(&self, call: NewToolCall) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| record_tx(tx, &call).map(|_| ()))
            .await
    }
}

/// Write `call` in the caller's transaction. `false` when a row for its
/// `event_id` already exists (a redelivered event).
pub fn record_tx(conn: &rusqlite::Connection, call: &NewToolCall) -> Result<bool, DomainError> {
    let at = crate::database::ts_to_string(call.at.unwrap_or_else(oxplow_domain::Timestamp::now));
    let n = conn
        .execute(
            "INSERT INTO agent_tool_call
               (thread_id, effort_id, turn_id, tool, path, detail, ok, at, event_id, kind,
                agent_session_id, subagent_id, subagent_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING",
            rusqlite::params![
                call.thread_id,
                call.effort_id,
                call.turn_id,
                call.tool,
                call.path,
                call.detail,
                call.ok.map(i64::from),
                at,
                call.event_id,
                call.kind,
                call.agent_session_id,
                call.subagent_id,
                call.subagent_kind,
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    async fn seeded() -> (SqliteToolCallStore, SemanticLayer) {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'T', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 1, 'Task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at) VALUES (1, 'work_item:oxplow:tsk1', 1, '2026-01-01');",
            )
        })
        .await
        .unwrap();
        (SqliteToolCallStore::new(db.clone()), SemanticLayer::new(db))
    }

    /// A call of `kind` (oxplow's vocabulary), named by its kind.
    fn call(kind: &str, path: Option<&str>, ok: Option<bool>) -> NewToolCall {
        NewToolCall {
            thread_id: 1,
            effort_id: Some(1),
            tool: kind.into(),
            kind: kind.into(),
            path: path.map(str::to_string),
            detail: None,
            ok,
            ..Default::default()
        }
    }

    /// P3.2 (tsk472): the row is a projection of `agent.tool.finished`, so
    /// a redelivered event writes nothing.
    /// The session and the subagent a call ran in are recorded and read
    /// back through `v_tool_call`.
    #[tokio::test]
    async fn a_call_carries_its_session_and_subagent() {
        let (store, sl) = seeded().await;
        store
            .record(NewToolCall {
                agent_session_id: Some(3),
                subagent_id: Some("afd8".into()),
                subagent_kind: Some("Explore".into()),
                ..call("read", Some("a.rs"), Some(true))
            })
            .await
            .unwrap();
        let rows = sl
            .query_sql(
                "SELECT agent_session_id, subagent_id, subagent_kind FROM v_tool_call",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            json!([[3, "afd8", "Explore"]])
        );
    }

    #[tokio::test]
    async fn record_tx_projects_each_event_once() {
        let (store, sl) = seeded().await;
        let call = NewToolCall {
            event_id: Some("evt-1".into()),
            turn_id: None,
            ..call("read", Some("src/a.rs"), Some(true))
        };
        let first = store
            .db
            .transaction({
                let call = call.clone();
                move |tx| record_tx(tx, &call)
            })
            .await
            .unwrap();
        let again = store
            .db
            .transaction(move |tx| record_tx(tx, &call))
            .await
            .unwrap();
        assert_eq!((first, again), (true, false));
        let rows = sl
            .query_sql("SELECT event_id, turn_id FROM v_tool_call", vec![], None)
            .await
            .unwrap()
            .rows;
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            json!([["evt-1", null]])
        );
    }

    #[tokio::test]
    async fn context_reads_and_struggle_are_derived() {
        let (store, sl) = seeded().await;
        store
            .record(call("read", Some(".context/usability.md"), Some(true)))
            .await
            .unwrap();
        store
            .record(call("read", Some("src/main.rs"), Some(true)))
            .await
            .unwrap();
        for _ in 0..5 {
            store
                .record(call("edit", Some("src/hot.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..4 {
            store
                .record(call("edit", Some("src/calm.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..3 {
            store
                .record(call("shell", None, Some(false)))
                .await
                .unwrap();
        }
        store.record(call("shell", None, None)).await.unwrap(); // unknown outcome isn't a failure

        let rows = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            rows("SELECT path FROM v_context_read").await,
            json!([[".context/usability.md"]])
        );
        assert_eq!(
            rows("SELECT kind, subject, count FROM v_struggle ORDER BY kind").await,
            json!([
                ["failed_commands", "shell", 3],
                ["repeated_edits", "src/hot.rs", 5]
            ])
        );
        assert_eq!(
            rows("SELECT count(*) FROM v_tool_call").await,
            json!([[15]])
        );
    }
}
