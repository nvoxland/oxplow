//! Records of oxplow's own model calls (`ai_call` → `v_ai_call`), each
//! with what it was asked and what it answered: bodies kept by content
//! hash in `event_content` (namespace [`BODY_NAMESPACE`], under its
//! retention window). See `.context/ai-providers.md`.

use oxplow_domain::DomainError;

use crate::Database;

/// The `event_content` namespace a call's request and response are kept
/// under (its retention window is core's `ai`).
pub const BODY_NAMESPACE: &str = "ai";

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
    /// What the model was asked: `{ system, prompt, json }` or `{ state,
    /// questions }`.
    pub request: serde_json::Value,
    /// What it answered (`{ text }` or `{ answers }`); `None` when the
    /// call failed.
    pub response: Option<serde_json::Value>,
}

/// A body's bytes: its JSON serialized canonically (object keys sorted at
/// every depth), whole — a request is the cache key of what it computes
/// (`ai_result.request_hash`), so it is never cut.
fn body_bytes(body: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&crate::event_content_store::canonical(body)).expect("JSON serializes")
}

/// The content hash of `request`: what a call recording it stores it
/// under, and what a recorded result is keyed by.
pub fn request_hash(request: &serde_json::Value) -> String {
    crate::event_content_store::hash(&body_bytes(request))
}

#[derive(Clone)]
pub struct SqliteAiCallStore {
    db: Database,
}

impl SqliteAiCallStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Record a call, its request and its response; its row id.
    pub async fn record(&self, call: NewAiCall) -> Result<i64, DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |c| {
                let put = |body: &serde_json::Value| {
                    crate::event_content_store::put_tx(c, BODY_NAMESPACE, &body_bytes(body))
                        .map(|r| r.hash)
                };
                let request = put(&call.request)?;
                let response = call.response.as_ref().map(put).transpose()?;
                c.execute(
                    "INSERT INTO ai_call (role, provider, model, caller, input_tokens, output_tokens,
                                          latency_ms, ok, error, at, input_hash, request_hash,
                                          response_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                        call.input_hash,
                        request,
                        response
                    ],
                )
                .map_err(crate::map_sql_err)?;
                Ok(c.last_insert_rowid())
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
            request: serde_json::json!({ "system": null, "prompt": "hi", "json": false }),
            response: Some(serde_json::json!({ "text": "hello" })),
        }
    }

    /// A call keeps what it was asked and answered, by content hash: the
    /// same request is one body; a failed call has no response.
    #[tokio::test]
    async fn a_call_keeps_its_request_and_response() {
        let db = Database::in_memory();
        let store = SqliteAiCallStore::new(db.clone());
        let ok = store.record(call("main")).await.unwrap();
        let failed = store
            .record(NewAiCall {
                ok: false,
                error: Some("rate limited".into()),
                response: None,
                ..call("main")
            })
            .await
            .unwrap();
        let bodies = |id: i64| {
            let db = db.clone();
            async move {
                db.read(move |c| {
                    c.query_row(
                        "SELECT request_hash, response_hash FROM ai_call WHERE id = ?1",
                        [id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .map_err(crate::map_sql_err)
                })
                .await
                .unwrap()
            }
        };
        let (rq, rs) = bodies(ok).await;
        assert_eq!(rq, request_hash(&call("main").request));
        let text = |h: String| {
            let db = db.clone();
            async move {
                String::from_utf8(
                    crate::event_content_store::read(&db, &h)
                        .await
                        .unwrap()
                        .unwrap(),
                )
                .unwrap()
            }
        };
        assert_eq!(
            text(rq.clone()).await,
            r#"{"json":false,"prompt":"hi","system":null}"#
        );
        assert_eq!(text(rs.unwrap()).await, r#"{"text":"hello"}"#);
        assert_eq!(bodies(failed).await, (rq, None));
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
