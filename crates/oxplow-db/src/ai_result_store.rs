//! Recorded AI computations (`ai_result` → `v_ai_result`, P5.E1): a
//! result kept by `(input_hash, provider, model, request_hash)` — the
//! request being the exact prompt the model was sent, so a changed prompt
//! is a new result. See `.context/ai-providers.md` "Recorded computations".

use oxplow_domain::DomainError;
use rusqlite::{Connection, OptionalExtension};

use crate::Database;

#[derive(Debug, Clone, PartialEq)]
pub struct NewAiResult {
    pub input_hash: String,
    /// The provider that served `model` (a model name alone doesn't say
    /// what answered).
    pub provider: String,
    pub model: String,
    /// The content hash of the request the model was sent
    /// (`ai_call.request_hash`).
    pub request_hash: String,
    pub op: String,
    pub role: String,
    pub caller: String,
    pub output: serde_json::Value,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub ai_call_id: Option<i64>,
}

/// A recorded result.
#[derive(Debug, Clone, PartialEq)]
pub struct AiResult {
    pub output: serde_json::Value,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub ai_call_id: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteAiResultStore {
    db: Database,
}

/// The result recorded for this key, read in `c`.
fn get_in(
    c: &Connection,
    input_hash: &str,
    provider: &str,
    model: &str,
    request_hash: &str,
) -> Result<Option<AiResult>, DomainError> {
    let row = c
        .query_row(
            "SELECT output_json, input_tokens, output_tokens, ai_call_id FROM ai_result
             WHERE input_hash = ?1 AND provider = ?2 AND model = ?3 AND request_hash = ?4",
            rusqlite::params![input_hash, provider, model, request_hash],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| DomainError::Storage(e.to_string()))?;
    row.map(|(json, input_tokens, output_tokens, ai_call_id)| {
        serde_json::from_str(&json)
            .map(|output| AiResult {
                output,
                input_tokens,
                output_tokens,
                ai_call_id,
            })
            .map_err(|e| DomainError::Storage(format!("ai_result output: {e}")))
    })
    .transpose()
}

impl SqliteAiResultStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The result recorded for this input, provider, model and request.
    pub async fn get(
        &self,
        input_hash: &str,
        provider: &str,
        model: &str,
        request_hash: &str,
    ) -> Result<Option<AiResult>, DomainError> {
        let key = [input_hash, provider, model, request_hash].map(str::to_string);
        self.db
            .read(move |c| get_in(c, &key[0], &key[1], &key[2], &key[3]))
            .await
    }

    /// Record a result and return what is recorded for its key: this one,
    /// or — when a concurrent computation recorded first — that one, so
    /// one question always reads one answer.
    pub async fn insert(&self, result: NewAiResult) -> Result<AiResult, DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO ai_result (input_hash, provider, model, request_hash, op, role,
                                            caller, output_json, input_tokens, output_tokens, at,
                                            ai_call_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT (input_hash, provider, model, request_hash) DO NOTHING",
                    rusqlite::params![
                        result.input_hash,
                        result.provider,
                        result.model,
                        result.request_hash,
                        result.op,
                        result.role,
                        result.caller,
                        result.output.to_string(),
                        result.input_tokens,
                        result.output_tokens,
                        at,
                        result.ai_call_id,
                    ],
                )
                .map_err(|e| DomainError::Storage(e.to_string()))?;
                get_in(
                    c,
                    &result.input_hash,
                    &result.provider,
                    &result.model,
                    &result.request_hash,
                )?
                .ok_or_else(|| DomainError::Storage("ai_result: the recorded row is gone".into()))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A result is kept by its input, provider, model and request; a
    /// second insert for the same key — a concurrent computation — keeps
    /// the first and returns it, so one question always reads one answer.
    #[tokio::test]
    async fn a_result_is_kept_by_input_provider_model_and_request() {
        let db = Database::in_memory();
        let calls = crate::SqliteAiCallStore::new(db.clone());
        for _ in 0..3 {
            calls
                .record(crate::NewAiCall {
                    role: "decide".into(),
                    provider: "p".into(),
                    model: "m".into(),
                    caller: "test".into(),
                    input_tokens: 3,
                    output_tokens: 1,
                    latency_ms: 1,
                    ok: true,
                    error: None,
                    input_hash: Some("h".into()),
                    request: serde_json::json!({}),
                    response: None,
                })
                .await
                .unwrap();
        }
        let store = SqliteAiResultStore::new(db);
        let result = |label: &str, call: i64| NewAiResult {
            input_hash: "h".into(),
            provider: "p".into(),
            model: "m".into(),
            request_hash: "rq".into(),
            op: "classify".into(),
            role: "decide".into(),
            caller: "test".into(),
            output: serde_json::json!({ "label": label }),
            input_tokens: 3,
            output_tokens: 1,
            ai_call_id: Some(call),
        };
        assert_eq!(store.get("h", "p", "m", "rq").await.unwrap(), None);
        let first = store.insert(result("bug", 1)).await.unwrap();
        assert_eq!(first.output["label"], "bug");
        let second = store.insert(result("feature", 2)).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(second.ai_call_id, Some(1));
        let got = store.get("h", "p", "m", "rq").await.unwrap().unwrap();
        assert_eq!(got, first);
        assert_eq!(store.get("h", "p", "other", "rq").await.unwrap(), None);
        assert_eq!(store.get("h", "p", "m", "rq2").await.unwrap(), None);
        // Another provider serving the same model name is another result.
        assert_eq!(store.get("h", "q", "m", "rq").await.unwrap(), None);
        let q = store
            .insert(NewAiResult {
                provider: "q".into(),
                ..result("feature", 3)
            })
            .await
            .unwrap();
        assert_eq!(q.output["label"], "feature");
    }
}
