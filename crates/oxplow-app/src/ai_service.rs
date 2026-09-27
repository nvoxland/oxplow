//! Oxplow's model calls, by role. Resolves a role to a provider and model
//! from the user-global `ai.yaml` (plus any project overrides), fetches the
//! provider's key from the keychain, refuses calls over the role's daily
//! budget, and records every call in `ai_call` (`v_ai_call`).
//! See `.context/ai-providers.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use oxplow_ai::client::{AiError, Client, Completion, Decision, Question};
use oxplow_ai::config::{AiConfig, ProviderConfig, Role, RoleBinding};
use oxplow_ai::secrets::SecretStore;
use oxplow_db::{NewAiCall, SqliteAiCallStore};
use parking_lot::RwLock;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AiServiceError {
    #[error("no model is assigned to the `{0}` role (Settings → AI)")]
    NotConfigured(String),
    #[error("the `{role}` role has spent ${spent:.2} of its ${budget:.2} daily budget")]
    OverBudget {
        role: String,
        spent: f64,
        budget: f64,
    },
    #[error("AI settings: {0}")]
    Config(String),
    #[error("keychain: {0}")]
    Secret(String),
    #[error(transparent)]
    Call(#[from] AiError),
}

pub struct AiService {
    client: Client,
    secrets: Arc<dyn SecretStore>,
    calls: Arc<SqliteAiCallStore>,
    /// Where `ai.yaml` lives; `None` when there's no config dir, which
    /// leaves every role unconfigured.
    config_dir: Option<PathBuf>,
    overrides: RwLock<BTreeMap<Role, RoleBinding>>,
}

impl AiService {
    pub fn new(
        client: Client,
        secrets: Arc<dyn SecretStore>,
        calls: Arc<SqliteAiCallStore>,
        config_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            client,
            secrets,
            calls,
            config_dir,
            overrides: RwLock::default(),
        }
    }

    /// Replace the project's role overrides.
    pub fn set_overrides(&self, overrides: BTreeMap<Role, RoleBinding>) {
        *self.overrides.write() = overrides;
    }

    /// The effective configuration: global `ai.yaml` plus project overrides.
    pub fn config(&self) -> Result<AiConfig, AiServiceError> {
        let global = match &self.config_dir {
            Some(dir) => AiConfig::load(dir).map_err(|e| AiServiceError::Config(e.to_string()))?,
            None => AiConfig::default(),
        };
        Ok(global.with_overrides(&self.overrides.read()))
    }

    /// Generate text with the model `role` is bound to. `caller` names who
    /// asked (an extension, `mcp`, core), for the usage record.
    pub async fn complete(
        &self,
        role: Role,
        caller: &str,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<Completion, AiServiceError> {
        let (provider, binding, key) = self.prepare(role).await?;
        let started = Instant::now();
        let result = self
            .client
            .complete(
                &provider,
                key.as_deref(),
                &binding.model,
                system,
                prompt,
                json,
            )
            .await;
        let outcome = result
            .as_ref()
            .map(|c| (c.input_tokens, c.output_tokens, c.cost_usd));
        self.record(role, caller, &provider, &binding, started, outcome)
            .await;
        Ok(result?)
    }

    /// Answer typed questions about `state` with the model `role` is bound to.
    pub async fn decide(
        &self,
        role: Role,
        caller: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Decision, AiServiceError> {
        let (provider, binding, key) = self.prepare(role).await?;
        let started = Instant::now();
        let result = self
            .client
            .decide(&provider, key.as_deref(), &binding.model, state, questions)
            .await;
        let outcome = result
            .as_ref()
            .map(|d| (d.input_tokens, d.output_tokens, d.cost_usd));
        self.record(role, caller, &provider, &binding, started, outcome)
            .await;
        Ok(result?)
    }

    /// Resolve `role`, check its budget, and fetch its provider's key.
    /// Keys are stored under the provider id; a provider without one (a
    /// local server) is called without auth.
    async fn prepare(
        &self,
        role: Role,
    ) -> Result<(ProviderConfig, RoleBinding, Option<String>), AiServiceError> {
        let config = self.config()?;
        let (provider, binding) = config
            .resolve(role)
            .map(|(p, b)| (p.clone(), b.clone()))
            .ok_or_else(|| AiServiceError::NotConfigured(role_name(role)))?;
        if let Some(budget) = binding.daily_budget_usd {
            let spent = self
                .calls
                .spent_since(&role_name(role), &start_of_today())
                .await
                .map_err(|e| AiServiceError::Config(e.to_string()))?;
            if spent >= budget {
                return Err(AiServiceError::OverBudget {
                    role: role_name(role),
                    spent,
                    budget,
                });
            }
        }
        let key = self
            .secrets
            .get(&provider.id)
            .map_err(|e| AiServiceError::Secret(e.to_string()))?;
        Ok((provider, binding, key))
    }

    async fn record(
        &self,
        role: Role,
        caller: &str,
        provider: &ProviderConfig,
        binding: &RoleBinding,
        started: Instant,
        outcome: Result<(i64, i64, Option<f64>), &AiError>,
    ) {
        let (input_tokens, output_tokens, cost_usd) = outcome.unwrap_or((0, 0, None));
        let error = outcome.err();
        let call = NewAiCall {
            role: role_name(role),
            provider: provider.id.clone(),
            model: binding.model.clone(),
            caller: caller.to_string(),
            input_tokens,
            output_tokens,
            latency_ms: started.elapsed().as_millis() as i64,
            cost_usd,
            ok: error.is_none(),
            error: error.map(|e| e.to_string()),
        };
        if let Err(e) = self.calls.record(call).await {
            tracing::warn!(error = %e, "failed to record an AI call");
        }
    }
}

/// The role's name as `ai.yaml` and `v_ai_call` spell it.
fn role_name(role: Role) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Midnight UTC today, in the RFC 3339 form `ai_call.at` is stored in, so
/// the two compare as strings.
fn start_of_today() -> String {
    let now = oxplow_domain::Timestamp::now().unix_ms();
    let midnight = oxplow_domain::Timestamp::from_unix_ms(now - now.rem_euclid(86_400_000));
    serde_json::to_value(midnight)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::client::Answer;
    use oxplow_ai::config::ProviderKind;
    use oxplow_ai::secrets::MemorySecrets;
    use oxplow_ai::testing::mock;
    use oxplow_db::Database;
    use serde_json::json;

    fn chat_reply(cost: f64) -> serde_json::Value {
        json!({"choices": [{"message": {"content": "hello"}}],
               "usage": {"prompt_tokens": 10, "completion_tokens": 5, "cost": cost}})
    }

    /// A service whose `summarize` role points at `base` (an OpenRouter-kind
    /// provider, so the mock's `usage.cost` is read), with `budget`.
    fn service(base: &str, budget: Option<f64>, dir: &tempfile::TempDir) -> (AiService, Database) {
        let cfg = AiConfig {
            providers: vec![ProviderConfig {
                id: "or".into(),
                kind: ProviderKind::Openrouter,
                base_url: Some(base.into()),
            }],
            roles: BTreeMap::from([(
                Role::Summarize,
                RoleBinding {
                    provider: "or".into(),
                    model: "m".into(),
                    daily_budget_usd: budget,
                },
            )]),
        };
        cfg.save(dir.path()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        secrets.set("or", "sk-1").unwrap();
        let db = Database::in_memory();
        let calls = Arc::new(SqliteAiCallStore::new(db.clone()));
        (
            AiService::new(Client::default(), secrets, calls, Some(dir.path().into())),
            db,
        )
    }

    /// `v_ai_call` rows as `[role, caller, ok, error is set]`.
    async fn recorded(db: &Database) -> serde_json::Value {
        let out = oxplow_db::SemanticLayer::new(db.clone())
            .query_sql(
                "SELECT role, caller, ok, error IS NOT NULL, cost_usd FROM v_ai_call ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&out.rows).unwrap()
    }

    #[tokio::test]
    async fn a_call_uses_the_role_binding_and_keychain_key_and_is_recorded() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply(0.25)).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, None, &dir);
        let c = svc
            .complete(Role::Summarize, "ext:review", None, "hi", false)
            .await
            .unwrap();
        assert_eq!(c.text, "hello");
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer sk-1");
        assert_eq!(body["model"], "m");
        assert_eq!(
            recorded(&db).await,
            json!([["summarize", "ext:review", 1, 0, 0.25]])
        );
    }

    #[tokio::test]
    async fn an_unassigned_role_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://127.0.0.1:1", None, &dir);
        let err = svc
            .complete(Role::Main, "core", None, "hi", false)
            .await
            .unwrap_err();
        assert_eq!(err, AiServiceError::NotConfigured("main".into()));
    }

    #[tokio::test]
    async fn project_overrides_win_over_the_global_file() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply(0.0)).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(&base, None, &dir);
        svc.set_overrides(BTreeMap::from([(
            Role::Main,
            RoleBinding {
                provider: "or".into(),
                model: "big".into(),
                daily_budget_usd: None,
            },
        )]));
        svc.complete(Role::Main, "core", None, "hi", false)
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap()[0].2["model"], "big");
    }

    #[tokio::test]
    async fn calls_stop_once_the_daily_budget_is_spent() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply(0.6)).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(&base, Some(1.0), &dir);
        svc.complete(Role::Summarize, "core", None, "a", false)
            .await
            .unwrap();
        svc.complete(Role::Summarize, "core", None, "b", false)
            .await
            .unwrap();
        let err = svc
            .complete(Role::Summarize, "core", None, "c", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AiServiceError::OverBudget { .. }), "{err:?}");
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "the third call never reached the provider"
        );
    }

    #[tokio::test]
    async fn failed_calls_are_recorded_too() {
        let (base, _) = mock("/chat/completions", 401, json!({})).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, None, &dir);
        let err = svc
            .complete(Role::Summarize, "core", None, "a", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AiServiceError::Call(AiError::Auth { .. })));
        assert_eq!(
            recorded(&db).await,
            json!([["summarize", "core", 0, 1, null]])
        );
    }

    #[tokio::test]
    async fn decide_goes_through_the_same_path() {
        let reply = json!({"answers": {"ok": {"type": "noul", "probability": 0.9}}});
        let (base, _) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(&base, None, &dir);
        let qs = BTreeMap::from([(
            "ok".to_string(),
            Question::Noul {
                instructions: "Is it ok?".into(),
            },
        )]);
        let d = svc
            .decide(Role::Summarize, "core", "state", &qs)
            .await
            .unwrap();
        assert_eq!(d.answers["ok"], Answer::Noul { probability: 0.9 });
    }
}
