//! Records of oxplow's own model calls (`ai_call` → `v_ai_call`). See
//! `.context/ai-providers.md`.

use oxplow_domain::DomainError;

use crate::Database;

#[derive(Debug, Clone, PartialEq)]
pub struct NewAiCall {
    pub role: String,
    pub provider: String,
    pub model: String,
    pub caller: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: i64,
    pub ok: bool,
    pub error: Option<String>,
    /// For a recorded computation, the hash of its input.
    pub input_hash: Option<String>,
}

#[derive(Clone)]
pub struct SqliteAiCallStore {
    db: Database,
}

impl SqliteAiCallStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Record a call; its row id.
    pub async fn record(&self, call: NewAiCall) -> Result<i64, DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO ai_call (role, provider, model, caller, input_tokens, output_tokens, latency_ms, ok, error, at, input_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        call.role,
                        call.provider,
                        call.model,
                        call.caller,
                        call.input_tokens,
                        call.output_tokens,
                        call.latency_ms,
                        i64::from(call.ok),
                        call.error,
                        at,
                        call.input_hash
                    ],
                )
                .map(|_| c.last_insert_rowid())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;

    fn call(role: &str) -> NewAiCall {
        NewAiCall {
            role: role.into(),
            provider: "p".into(),
            model: "m".into(),
            caller: "test".into(),
            input_tokens: 10,
            output_tokens: 5,
            latency_ms: 42,
            ok: true,
            error: None,
            input_hash: None,
        }
    }

    #[tokio::test]
    async fn records_calls_into_v_ai_call() {
        let db = Database::in_memory();
        let store = SqliteAiCallStore::new(db.clone());
        store.record(call("summarize")).await.unwrap();
        store.record(call("summarize")).await.unwrap();
        store.record(call("decide")).await.unwrap();
        let out = SemanticLayer::new(db)
            .query_sql(
                "SELECT role, count(*), sum(input_tokens) FROM v_ai_call GROUP BY role ORDER BY role",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["decide", 1, 10], ["summarize", 2, 20]])
        );
    }
}
