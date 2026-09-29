//! Persisted agent tool calls (`agent_tool_call`), recorded from the
//! PostToolUse hook. Read through `v_tool_call`, `v_context_read` and
//! `v_struggle`. See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;

use crate::Database;

#[derive(Debug, Clone, PartialEq)]
pub struct NewToolCall {
    pub thread_id: i64,
    pub effort_id: Option<i64>,
    pub tool: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ok: Option<bool>,
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
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO agent_tool_call (thread_id, effort_id, tool, path, detail, ok, at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        call.thread_id,
                        call.effort_id,
                        call.tool,
                        call.path,
                        call.detail,
                        call.ok.map(i64::from),
                        at
                    ],
                )
                .map(|_| ())
            })
            .await
    }
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

    fn call(tool: &str, path: Option<&str>, ok: Option<bool>) -> NewToolCall {
        NewToolCall {
            thread_id: 1,
            effort_id: Some(1),
            tool: tool.into(),
            path: path.map(str::to_string),
            detail: None,
            ok,
        }
    }

    #[tokio::test]
    async fn context_reads_and_struggle_are_derived() {
        let (store, sl) = seeded().await;
        store
            .record(call("Read", Some(".context/usability.md"), Some(true)))
            .await
            .unwrap();
        store
            .record(call("Read", Some("src/main.rs"), Some(true)))
            .await
            .unwrap();
        for _ in 0..5 {
            store
                .record(call("Edit", Some("src/hot.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..4 {
            store
                .record(call("Edit", Some("src/calm.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..3 {
            store.record(call("Bash", None, Some(false))).await.unwrap();
        }
        store.record(call("Bash", None, None)).await.unwrap(); // unknown outcome isn't a failure

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
                ["failed_commands", "Bash", 3],
                ["repeated_edits", "src/hot.rs", 5]
            ])
        );
        assert_eq!(
            rows("SELECT count(*) FROM v_tool_call").await,
            json!([[15]])
        );
    }
}
