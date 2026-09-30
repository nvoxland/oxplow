//! Recorded AI computations (`ai_result` → `v_ai_result`, P5.E1): a
//! result kept by `(input_hash, provider, model, prompt_version)`. See
//! `.context/ai-providers.md` "Recorded computations".

use oxplow_domain::DomainError;

use crate::Database;

#[derive(Debug, Clone, PartialEq)]
pub struct NewAiResult {
    pub input_hash: String,
    /// The provider that served `model` (a model name alone doesn't say
    /// what answered).
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
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

impl SqliteAiResultStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The result recorded for this input, provider, model and prompt
    /// version.
    pub async fn get(
        &self,
        input_hash: &str,
        provider: &str,
        model: &str,
        prompt_version: &str,
    ) -> Result<Option<AiResult>, DomainError> {
        let key = (
            input_hash.to_string(),
            provider.to_string(),
            model.to_string(),
            prompt_version.to_string(),
        );
        self.db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT output_json, input_tokens, output_tokens, ai_call_id FROM ai_result
                     WHERE input_hash = ?1 AND provider = ?2 AND model = ?3
                       AND prompt_version = ?4",
                    rusqlite::params![key.0, key.1, key.2, key.3],
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
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await?
            .map(|(json, input_tokens, output_tokens, ai_call_id)| {
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

    /// Record a result. A result already recorded for the same key (a
    /// concurrent computation) is kept.
    pub async fn insert(&self, result: NewAiResult) -> Result<(), DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO ai_result (input_hash, provider, model, prompt_version, op, role,
                                            caller, output_json, input_tokens, output_tokens, at,
                                            ai_call_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT (input_hash, provider, model, prompt_version) DO NOTHING",
                    rusqlite::params![
                        result.input_hash,
                        result.provider,
                        result.model,
                        result.prompt_version,
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
                .map(|_| ())
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_result_is_kept_by_input_provider_model_and_prompt_version() {
        let db = Database::in_memory();
        let store = SqliteAiResultStore::new(db);
        let result = |label: &str| NewAiResult {
            input_hash: "h".into(),
            provider: "p".into(),
            model: "m".into(),
            prompt_version: "classify@1".into(),
            op: "classify".into(),
            role: "decide".into(),
            caller: "test".into(),
            output: serde_json::json!({ "label": label }),
            input_tokens: 3,
            output_tokens: 1,
            ai_call_id: None,
        };
        assert_eq!(store.get("h", "p", "m", "classify@1").await.unwrap(), None);
        store.insert(result("bug")).await.unwrap();
        // A second insert for the same key keeps the first.
        store.insert(result("feature")).await.unwrap();
        let got = store
            .get("h", "p", "m", "classify@1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.output["label"], "bug");
        assert_eq!(
            store.get("h", "p", "other", "classify@1").await.unwrap(),
            None
        );
        assert_eq!(store.get("h", "p", "m", "classify@2").await.unwrap(), None);
        // Another provider serving the same model name is another result.
        assert_eq!(store.get("h", "q", "m", "classify@1").await.unwrap(), None);
        store
            .insert(NewAiResult {
                provider: "q".into(),
                ..result("feature")
            })
            .await
            .unwrap();
        let q = store
            .get("h", "q", "m", "classify@1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(q.output["label"], "feature");
    }
}
