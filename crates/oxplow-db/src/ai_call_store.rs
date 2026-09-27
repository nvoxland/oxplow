//! Records of oxplow's own model calls (`ai_call` → `v_ai_call`), and
//! the per-role spend that daily budgets check. See
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
    pub cost_usd: Option<f64>,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct SqliteAiCallStore {
    db: Database,
}

impl SqliteAiCallStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(&self, call: NewAiCall) -> Result<(), DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO ai_call (role, provider, model, caller, input_tokens, output_tokens, latency_ms, cost_usd, ok, error, at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        call.role,
                        call.provider,
                        call.model,
                        call.caller,
                        call.input_tokens,
                        call.output_tokens,
                        call.latency_ms,
                        call.cost_usd,
                        i64::from(call.ok),
                        call.error,
                        at
                    ],
                )
                .map(|_| ())
            })
            .await
    }

    /// Estimated USD spent by `role` since `since` (RFC 3339). Calls with
    /// an unknown price count as 0.
    pub async fn spent_since(&self, role: &str, since: &str) -> Result<f64, DomainError> {
        let (role, since) = (role.to_string(), since.to_string());
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0) FROM ai_call WHERE role = ?1 AND at >= ?2",
                    rusqlite::params![role, since],
                    |r| r.get(0),
                )
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;

    fn call(role: &str, cost: Option<f64>) -> NewAiCall {
        NewAiCall {
            role: role.into(),
            provider: "p".into(),
            model: "m".into(),
            caller: "test".into(),
            input_tokens: 10,
            output_tokens: 5,
            latency_ms: 42,
            cost_usd: cost,
            ok: true,
            error: None,
        }
    }

    #[tokio::test]
    async fn records_calls_and_sums_spend_per_role() {
        let db = Database::in_memory();
        let store = SqliteAiCallStore::new(db.clone());
        store.record(call("summarize", Some(0.25))).await.unwrap();
        store.record(call("summarize", Some(0.5))).await.unwrap();
        store.record(call("summarize", None)).await.unwrap(); // unknown price counts as 0
        store.record(call("decide", Some(9.0))).await.unwrap();
        let spent = store
            .spent_since("summarize", "2000-01-01T00:00:00Z")
            .await
            .unwrap();
        assert!((spent - 0.75).abs() < 1e-9, "{spent}");
        assert_eq!(
            store
                .spent_since("summarize", "2999-01-01T00:00:00Z")
                .await
                .unwrap(),
            0.0
        );
        let out = SemanticLayer::new(db)
            .query_sql(
                "SELECT role, count(*) FROM v_ai_call GROUP BY role ORDER BY role",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["decide", 1], ["summarize", 3]])
        );
    }
}
