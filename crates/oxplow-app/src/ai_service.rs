//! Oxplow's model calls, by role. Resolves a role to a provider and model
//! from the user-global `ai.yaml` (plus any project overrides), fetches the
//! provider's key from the keychain, and records every call in `ai_call`
//! (`v_ai_call`).
//! See `.context/ai-providers.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use oxplow_ai::client::{AiError, Client};
/// Re-exported so the IPC and MCP adapters need only `oxplow-app`.
pub use oxplow_ai::client::{Answer, Completion, Decision, Question};
use oxplow_ai::config::AiConfig;
pub use oxplow_ai::config::{ProviderConfig, ProviderKind, Role, RoleBinding};
use oxplow_ai::secrets::SecretStore;
use oxplow_db::{NewAiCall, SqliteAiCallStore};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AiServiceError {
    #[error("no model is assigned to the `{0}` role (Settings → AI)")]
    NotConfigured(String),
    #[error("AI settings: {0}")]
    Config(String),
    #[error("keychain: {0}")]
    Secret(String),
    #[error(transparent)]
    Call(#[from] AiError),
}

/// A provider as the UI and agents see it: never its key.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: Option<String>,
    pub key_set: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleStatus {
    pub role: Role,
    /// `None` when the role has no model.
    pub binding: Option<RoleBinding>,
    /// The binding comes from the project, not the global `ai.yaml`.
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub providers: Vec<ProviderStatus>,
    /// Every role, in `Role::ALL` order.
    pub roles: Vec<RoleStatus>,
}

/// Who a call is for, as its `ai_call` row records it.
#[derive(Debug, Clone, Copy)]
pub struct CallSite<'a> {
    /// What asked (`mcp:ai_decide`, `inferred-decisions`, …).
    pub caller: &'a str,
    /// For a recorded computation (`ai_compute`), the hash of its input.
    pub input_hash: Option<&'a str>,
}

impl<'a> CallSite<'a> {
    pub fn new(caller: &'a str) -> Self {
        Self {
            caller,
            input_hash: None,
        }
    }
}

pub struct AiService {
    client: Client,
    secrets: Arc<dyn SecretStore>,
    calls: Arc<SqliteAiCallStore>,
    /// Where `ai.yaml` lives; `None` when there's no config dir, which
    /// leaves every role unconfigured.
    config_dir: Option<PathBuf>,
    /// The project's role assignments, read on every resolve so edits to
    /// `project.yaml` apply without syncing anything.
    overrides: OverridesSource,
}

/// Where project role overrides come from (the live project config).
pub type OverridesSource = Arc<dyn Fn() -> BTreeMap<Role, RoleBinding> + Send + Sync>;

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
            overrides: Arc::new(BTreeMap::new),
        }
    }

    /// Layer `source`'s role assignments (the project's) over `ai.yaml`.
    /// This machine's AI setup: the global `ai.yaml` and the OS keychain
    /// (read only here), recording calls into `calls`. What a check run
    /// outside the app uses (`OXPLOW_LIVE_ANSWERABILITY`).
    pub fn for_this_machine(calls: Arc<oxplow_db::SqliteAiCallStore>) -> Self {
        Self::new(
            Client::default(),
            Arc::new(oxplow_ai::secrets::KeychainSecrets),
            calls,
            oxplow_config::global_config_dir(),
        )
    }

    pub fn with_project_overrides(mut self, source: OverridesSource) -> Self {
        self.overrides = source;
        self
    }

    /// Providers (with whether each has a key; never the key) and every
    /// role with its binding. What Settings → AI and `list_ai_roles` show.
    pub fn settings(&self) -> Result<AiSettings, AiServiceError> {
        let config = self.config()?;
        let overrides = (self.overrides)();
        let providers = config
            .providers
            .iter()
            .map(|p| {
                Ok(ProviderStatus {
                    id: p.id.clone(),
                    kind: p.kind,
                    base_url: p.base_url.clone(),
                    key_set: self.raw_key(&p.id)?.is_some(),
                })
            })
            .collect::<Result<_, AiServiceError>>()?;
        let roles = Role::ALL
            .iter()
            .map(|r| RoleStatus {
                role: *r,
                binding: config.roles.get(r).cloned(),
                overridden: overrides.contains_key(r),
            })
            .collect();
        Ok(AiSettings { providers, roles })
    }

    /// Add or replace a provider in the global `ai.yaml`. A non-empty `key`
    /// is stored in the keychain; `None` leaves any stored key alone.
    pub fn save_provider(
        &self,
        provider: ProviderConfig,
        key: Option<String>,
    ) -> Result<(), AiServiceError> {
        let mut global = self.global()?;
        let id = provider.id.trim().to_string();
        let provider = ProviderConfig {
            id: id.clone(),
            ..provider
        };
        match global.providers.iter_mut().find(|p| p.id == id) {
            Some(existing) => *existing = provider,
            None => global.providers.push(provider),
        }
        self.save_global(&global)?;
        // The key is bound to the URL it's saved for (tsk346). This path is
        // the person's (Settings → AI; UI-only), so saving rebinds a kept
        // key to the provider's URL as it is now.
        let endpoint = endpoint_of(global.providers.iter().find(|p| p.id == id));
        let key = match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => Some(k.to_string()),
            None => self.raw_key(&id)?.map(|stored| stored.key),
        };
        if let Some(key) = key {
            let stored = StoredKey { key, endpoint };
            self.secrets
                .set(&id, &stored.encode())
                .map_err(|e| AiServiceError::Secret(e.to_string()))?;
        }
        Ok(())
    }

    /// Remove a provider and its key. Refused while a role still uses it.
    pub fn remove_provider(&self, id: &str) -> Result<(), AiServiceError> {
        let mut global = self.global()?;
        let users: Vec<String> = global
            .roles
            .iter()
            .filter(|(_, b)| b.provider == id)
            .map(|(r, _)| role_name(*r))
            .collect();
        if !users.is_empty() {
            return Err(AiServiceError::Config(format!(
                "`{id}` is used by {}; assign those roles elsewhere first",
                users.join(", ")
            )));
        }
        global.providers.retain(|p| p.id != id);
        self.save_global(&global)?;
        self.secrets
            .delete(id)
            .map_err(|e| AiServiceError::Secret(e.to_string()))
    }

    /// Assign a role to a provider + model in the global `ai.yaml`, or
    /// unassign it with `None`.
    pub fn set_role(&self, role: Role, binding: Option<RoleBinding>) -> Result<(), AiServiceError> {
        let mut global = self.global()?;
        match binding {
            Some(b) => global.roles.insert(role, b),
            None => global.roles.remove(&role),
        };
        self.save_global(&global)
    }

    /// Make one small call to `model` on provider `id` to check the key,
    /// URL and model name. Returns the model's reply. Not recorded: it
    /// isn't a role's work.
    pub async fn test_provider(&self, id: &str, model: &str) -> Result<String, AiServiceError> {
        let provider = self
            .config()?
            .providers
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AiServiceError::Config(format!("no provider `{id}`")))?;
        let key = self.key(&provider)?;
        if provider.kind == ProviderKind::Typesafe {
            let q = BTreeMap::from([(
                "ok".to_string(),
                Question::Noul {
                    instructions: "Is this a connection test?".into(),
                },
            )]);
            let d = self
                .client
                .decide(&provider, key.as_deref(), model, "A connection test.", &q)
                .await?;
            return Ok(match d.answers.get("ok") {
                Some(Answer::Noul { probability }) => {
                    format!("Answered (yes: {:.0}%)", probability * 100.0)
                }
                _ => "Answered".to_string(),
            });
        }
        let c = self
            .client
            .complete(
                &provider,
                key.as_deref(),
                model,
                None,
                "Reply with the single word OK.",
                false,
            )
            .await?;
        Ok(c.text.trim().chars().take(200).collect())
    }

    /// Summarize `text` with the `summarize` role. `instructions` say what
    /// to focus on.
    pub async fn summarize(
        &self,
        caller: &str,
        text: &str,
        instructions: Option<&str>,
    ) -> Result<String, AiServiceError> {
        let c = self
            .complete(
                Role::Summarize,
                caller,
                Some(&summarize_system(instructions)),
                text,
                false,
            )
            .await?;
        Ok(c.text.trim().to_string())
    }

    /// The effective configuration: global `ai.yaml` plus project overrides.
    pub fn config(&self) -> Result<AiConfig, AiServiceError> {
        let global = match &self.config_dir {
            Some(dir) => AiConfig::load(dir).map_err(|e| AiServiceError::Config(e.to_string()))?,
            None => AiConfig::default(),
        };
        Ok(global.with_overrides(&(self.overrides)()))
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
        self.complete_as(role, CallSite::new(caller), system, prompt, json)
            .await
            .map(|(c, _)| c)
    }

    /// [`Self::complete`], recorded against `site`; the call's `ai_call`
    /// row id comes back with it.
    pub async fn complete_as(
        &self,
        role: Role,
        site: CallSite<'_>,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<(Completion, Option<i64>), AiServiceError> {
        let (provider, binding, key) = self.prepare(role)?;
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
        let outcome = result.as_ref().map(|c| (c.input_tokens, c.output_tokens));
        let call_id = self
            .record(role, site, &provider, &binding, started, outcome)
            .await;
        Ok((result?, call_id))
    }

    /// Answer typed questions about `state` with the model `role` is bound to.
    pub async fn decide(
        &self,
        role: Role,
        caller: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Decision, AiServiceError> {
        self.decide_as(role, CallSite::new(caller), state, questions)
            .await
            .map(|(d, _)| d)
    }

    /// [`Self::decide`], recorded against `site`; the call's `ai_call` row
    /// id comes back with it.
    pub async fn decide_as(
        &self,
        role: Role,
        site: CallSite<'_>,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<(Decision, Option<i64>), AiServiceError> {
        let (provider, binding, key) = self.prepare(role)?;
        let started = Instant::now();
        let result = self
            .client
            .decide(&provider, key.as_deref(), &binding.model, state, questions)
            .await;
        let outcome = result.as_ref().map(|d| (d.input_tokens, d.output_tokens));
        let call_id = self
            .record(role, site, &provider, &binding, started, outcome)
            .await;
        Ok((result?, call_id))
    }

    /// The model `role` is bound to now (what a recorded result is keyed
    /// by), without calling it.
    pub fn model_for(&self, role: Role) -> Result<String, AiServiceError> {
        self.config()?
            .resolve(role)
            .map(|(_, b)| b.model.clone())
            .ok_or_else(|| AiServiceError::NotConfigured(role_name(role)))
    }

    /// Resolve `role` and fetch its provider's key.
    /// Keys are stored under the provider id; a provider without one (a
    /// local server) is called without auth.
    fn prepare(
        &self,
        role: Role,
    ) -> Result<(ProviderConfig, RoleBinding, Option<String>), AiServiceError> {
        let config = self.config()?;
        let (provider, binding) = config
            .resolve(role)
            .map(|(p, b)| (p.clone(), b.clone()))
            .ok_or_else(|| AiServiceError::NotConfigured(role_name(role)))?;
        let key = self.key(&provider)?;
        Ok((provider, binding, key))
    }

    /// The stored key entry, whatever URL it's bound to.
    fn raw_key(&self, provider_id: &str) -> Result<Option<StoredKey>, AiServiceError> {
        Ok(self
            .secrets
            .get(provider_id)
            .map_err(|e| AiServiceError::Secret(e.to_string()))?
            .map(|raw| StoredKey::decode(&raw)))
    }

    /// The key to send to `provider`: only when it was saved for the
    /// provider's URL as it is now. `ai.yaml` is a plain file an agent can
    /// edit; pointing a provider elsewhere must not carry the key along.
    fn key(&self, provider: &ProviderConfig) -> Result<Option<String>, AiServiceError> {
        let Some(stored) = self.raw_key(&provider.id)? else {
            return Ok(None);
        };
        let now = endpoint_of(Some(provider));
        if stored.endpoint != now {
            let shown = |e: &str| {
                if e.is_empty() {
                    "its default URL".to_string()
                } else {
                    format!("`{e}`")
                }
            };
            return Err(AiServiceError::Config(format!(
                "the key for `{}` was saved for {}, but ai.yaml now points it at {}. \
                 If that's intended, re-save the provider in Settings → AI.",
                provider.id,
                shown(&stored.endpoint),
                shown(&now)
            )));
        }
        Ok(Some(stored.key))
    }

    fn config_dir(&self) -> Result<&std::path::Path, AiServiceError> {
        self.config_dir.as_deref().ok_or_else(|| {
            AiServiceError::Config("there's no config directory to save ai.yaml in".into())
        })
    }

    /// The global `ai.yaml` alone, without project overrides: what the
    /// settings writers edit.
    fn global(&self) -> Result<AiConfig, AiServiceError> {
        AiConfig::load(self.config_dir()?).map_err(|e| AiServiceError::Config(e.to_string()))
    }

    fn save_global(&self, config: &AiConfig) -> Result<(), AiServiceError> {
        config
            .save(self.config_dir()?)
            .map_err(|e| AiServiceError::Config(e.to_string()))
    }

    /// Record a call in `ai_call`; its row id (`None` when recording
    /// failed, which is logged, never an error).
    async fn record(
        &self,
        role: Role,
        site: CallSite<'_>,
        provider: &ProviderConfig,
        binding: &RoleBinding,
        started: Instant,
        outcome: Result<(i64, i64), &AiError>,
    ) -> Option<i64> {
        let (input_tokens, output_tokens) = outcome.unwrap_or((0, 0));
        let error = outcome.err();
        let call = NewAiCall {
            role: role_name(role),
            provider: provider.id.clone(),
            model: binding.model.clone(),
            caller: site.caller.to_string(),
            input_tokens,
            output_tokens,
            latency_ms: started.elapsed().as_millis() as i64,
            ok: error.is_none(),
            error: error.map(|e| e.to_string()),
            input_hash: site.input_hash.map(str::to_string),
        };
        match self.calls.record(call).await {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(error = %e, "failed to record an AI call");
                None
            }
        }
    }
}

/// `project.yaml`'s `ai.roles` as role bindings. Names were validated on
/// load; anything unrecognized is skipped.
pub fn project_overrides(
    roles: &BTreeMap<String, oxplow_config::AiRoleOverride>,
) -> BTreeMap<Role, RoleBinding> {
    roles
        .iter()
        .filter_map(|(name, o)| {
            let role: Role = serde_json::from_value(serde_json::json!(name)).ok()?;
            Some((
                role,
                RoleBinding {
                    provider: o.provider.clone(),
                    model: o.model.clone(),
                },
            ))
        })
        .collect()
}

/// The role's name as `ai.yaml` and `v_ai_call` spell it.
/// A keychain entry: the key and the endpoint it was saved for.
struct StoredKey {
    key: String,
    /// The provider's `base_url` when saved (normalized); empty = the
    /// kind's default URL.
    endpoint: String,
}

impl StoredKey {
    fn encode(&self) -> String {
        serde_json::json!({ "key": self.key, "endpoint": self.endpoint }).to_string()
    }

    /// A bare string is a key saved before binding existed: treat it as
    /// bound to the default URL, so a custom URL needs one re-save.
    fn decode(raw: &str) -> Self {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| {
                Some(StoredKey {
                    key: v.get("key")?.as_str()?.to_string(),
                    endpoint: v.get("endpoint")?.as_str()?.to_string(),
                })
            })
            .unwrap_or_else(|| StoredKey {
                key: raw.to_string(),
                endpoint: String::new(),
            })
    }
}

fn endpoint_of(provider: Option<&ProviderConfig>) -> String {
    provider
        .and_then(|p| p.base_url.as_deref())
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .unwrap_or_default()
}

/// The `summarize` role's system prompt, focused on `focus`.
pub fn summarize_system(focus: Option<&str>) -> String {
    let mut system = String::from(
        "Summarize the text you're given for a software developer. Be brief and concrete; \
         keep names, numbers and file paths exact. Reply with the summary only.",
    );
    if let Some(f) = focus.map(str::trim).filter(|f| !f.is_empty()) {
        system.push_str("\n\nFocus: ");
        system.push_str(f);
    }
    system
}

pub fn role_name(role: Role) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::secrets::MemorySecrets;
    use oxplow_ai::testing::mock;
    use oxplow_db::Database;
    use serde_json::json;

    fn chat_reply() -> serde_json::Value {
        json!({"choices": [{"message": {"content": "hello"}}],
               "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
    }

    /// A service whose `summarize` role points at `base`.
    fn service(base: &str, dir: &tempfile::TempDir) -> (AiService, Database) {
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
                },
            )]),
        };
        cfg.save(dir.path()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let bound = StoredKey {
            key: "sk-1".into(),
            endpoint: base.into(),
        };
        secrets.set("or", &bound.encode()).unwrap();
        let db = Database::in_memory();
        let calls = Arc::new(SqliteAiCallStore::new(db.clone()));
        (
            AiService::new(Client::default(), secrets, calls, Some(dir.path().into())),
            db,
        )
    }

    /// `v_ai_call` rows as `[role, caller, ok, error is set, input_tokens]`.
    async fn recorded(db: &Database) -> serde_json::Value {
        let out = crate::sql_gateway::SqlGateway::new(db.clone())
            .query_sql(
                "SELECT role, caller, ok, error IS NOT NULL, input_tokens FROM v_ai_call ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&out.rows).unwrap()
    }

    #[tokio::test]
    async fn a_call_uses_the_role_binding_and_keychain_key_and_is_recorded() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply()).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, &dir);
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
            json!([["summarize", "ext:review", 1, 0, 10]])
        );
    }

    #[tokio::test]
    async fn an_unassigned_role_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://127.0.0.1:1", &dir);
        let err = svc
            .complete(Role::Main, "core", None, "hi", false)
            .await
            .unwrap_err();
        assert_eq!(err, AiServiceError::NotConfigured("main".into()));
    }

    #[tokio::test]
    async fn project_overrides_win_over_the_global_file() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply()).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(&base, &dir);
        let svc = svc.with_project_overrides(Arc::new(|| {
            BTreeMap::from([(
                Role::Main,
                RoleBinding {
                    provider: "or".into(),
                    model: "big".into(),
                },
            )])
        }));
        svc.complete(Role::Main, "core", None, "hi", false)
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap()[0].2["model"], "big");
    }

    #[tokio::test]
    async fn failed_calls_are_recorded_too() {
        let (base, _) = mock("/chat/completions", 401, json!({})).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, &dir);
        let err = svc
            .complete(Role::Summarize, "core", None, "a", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AiServiceError::Call(AiError::Auth { .. })));
        assert_eq!(recorded(&db).await, json!([["summarize", "core", 0, 1, 0]]));
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
        let (svc, _) = service(&base, &dir);
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

    #[tokio::test]
    async fn settings_list_every_role_and_whether_keys_are_set_never_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://x", &dir);
        let st = svc.settings().unwrap();
        assert_eq!(st.providers.len(), 1);
        assert!(st.providers[0].key_set);
        assert!(!serde_json::to_string(&st).unwrap().contains("sk-1"));
        assert_eq!(st.roles.len(), Role::ALL.len());
        let summarize = st.roles.iter().find(|r| r.role == Role::Summarize).unwrap();
        assert_eq!(summarize.binding.as_ref().unwrap().model, "m");
        assert!(st
            .roles
            .iter()
            .find(|r| r.role == Role::Main)
            .unwrap()
            .binding
            .is_none());
    }

    #[tokio::test]
    async fn providers_and_roles_are_saved_to_ai_yaml_and_keys_to_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://x", &dir);
        svc.save_provider(
            ProviderConfig {
                id: "ts".into(),
                kind: ProviderKind::Typesafe,
                base_url: None,
            },
            Some("tk-9".into()),
        )
        .unwrap();
        svc.set_role(
            Role::Decide,
            Some(RoleBinding {
                provider: "ts".into(),
                model: "jev".into(),
            }),
        )
        .unwrap();
        let yaml = std::fs::read_to_string(dir.path().join("ai.yaml")).unwrap();
        assert!(yaml.contains("jev") && !yaml.contains("tk-9"), "{yaml}");
        assert_eq!(
            svc.raw_key("ts").unwrap().map(|k| k.key).as_deref(),
            Some("tk-9")
        );

        // Saving again without a key keeps the stored one.
        svc.save_provider(
            ProviderConfig {
                id: "ts".into(),
                kind: ProviderKind::Typesafe,
                base_url: Some("http://y".into()),
            },
            None,
        )
        .unwrap();
        assert_eq!(
            svc.raw_key("ts").unwrap().map(|k| k.key).as_deref(),
            Some("tk-9")
        );
        assert_eq!(svc.settings().unwrap().providers.len(), 2);

        // A provider in use can't be removed; once unassigned, it and its key go.
        let err = svc.remove_provider("ts").unwrap_err();
        assert!(err.to_string().contains("decide"), "{err}");
        svc.set_role(Role::Decide, None).unwrap();
        svc.remove_provider("ts").unwrap();
        assert_eq!(svc.secrets.get("ts").unwrap(), None);
        assert_eq!(svc.settings().unwrap().providers.len(), 1);
    }

    #[tokio::test]
    async fn invalid_settings_are_refused_and_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://x", &dir);
        let err = svc
            .set_role(
                Role::Main,
                Some(RoleBinding {
                    provider: "nope".into(),
                    model: "m".into(),
                }),
            )
            .unwrap_err();
        assert!(matches!(err, AiServiceError::Config(_)), "{err:?}");
        assert!(svc
            .settings()
            .unwrap()
            .roles
            .iter()
            .all(|r| r.role != Role::Main || r.binding.is_none()));
    }

    #[tokio::test]
    async fn testing_a_provider_makes_one_unrecorded_call() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply()).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, &dir);
        assert_eq!(
            svc.test_provider("or", "other-model").await.unwrap(),
            "hello"
        );
        assert_eq!(seen.lock().unwrap()[0].2["model"], "other-model");
        assert_eq!(recorded(&db).await, json!([]));
        assert!(svc.test_provider("missing", "m").await.is_err());
    }

    #[tokio::test]
    async fn overridden_roles_are_marked() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://x", &dir);
        let svc = svc.with_project_overrides(Arc::new(|| {
            BTreeMap::from([(
                Role::Fast,
                RoleBinding {
                    provider: "or".into(),
                    model: "small".into(),
                },
            )])
        }));
        let st = svc.settings().unwrap();
        let fast = st.roles.iter().find(|r| r.role == Role::Fast).unwrap();
        assert!(fast.overridden);
        assert_eq!(fast.binding.as_ref().unwrap().model, "small");
    }

    #[tokio::test]
    async fn testing_a_jev_provider_asks_a_typed_question() {
        let (base, _) = mock(
            "/v1/systemone",
            200,
            json!({"answers": {"ok": {"type": "noul", "noul": 0.93}}, "usage": {"input_tokens": 3, "output_tokens": 0}}),
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://x", &dir);
        svc.save_provider(
            ProviderConfig {
                id: "ts".into(),
                kind: ProviderKind::Typesafe,
                base_url: Some(base),
            },
            None,
        )
        .unwrap();
        assert_eq!(
            svc.test_provider("ts", "jev").await.unwrap(),
            "Answered (yes: 93%)"
        );
    }

    #[tokio::test]
    async fn summarize_uses_the_summarize_role_with_instructions() {
        let (base, seen) = mock("/chat/completions", 200, chat_reply()).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, db) = service(&base, &dir);
        let out = svc
            .summarize("mcp:ai_summarize", "long text", Some("focus on risks"))
            .await
            .unwrap();
        assert_eq!(out, "hello");
        let body = seen.lock().unwrap()[0].2.clone();
        let messages = body["messages"].to_string();
        assert!(
            messages.contains("long text") && messages.contains("focus on risks"),
            "{messages}"
        );
        assert_eq!(
            recorded(&db).await,
            json!([["summarize", "mcp:ai_summarize", 1, 0, 10]])
        );
    }

    #[test]
    fn project_config_role_names_match_the_roles() {
        let names: Vec<String> = Role::ALL.iter().map(|r| role_name(*r)).collect();
        assert_eq!(names, oxplow_config::AI_ROLE_NAMES.to_vec());
    }

    #[tokio::test]
    async fn project_yaml_roles_apply_and_follow_reloads() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let yaml = |model: &str| {
            std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
            std::fs::write(
                dir.path().join(".oxplow/project.yaml"),
                format!("ai:\n  roles:\n    fast: {{ provider: or, model: {model} }}\n"),
            )
            .unwrap();
        };
        yaml("small");
        let svc = crate::Services::in_memory(dir.path()).unwrap();
        svc.ai
            .save_provider(
                ProviderConfig {
                    id: "or".into(),
                    kind: ProviderKind::Openrouter,
                    base_url: None,
                },
                None,
            )
            .unwrap();
        let fast = |svc: &crate::Services| {
            let st = svc.ai.settings().unwrap();
            let r = st.roles.into_iter().find(|r| r.role == Role::Fast).unwrap();
            (r.overridden, r.binding.map(|b| b.model))
        };
        assert_eq!(fast(&svc), (true, Some("small".to_string())));
        yaml("tiny");
        svc.reload_config_from_disk().unwrap();
        assert_eq!(fast(&svc), (true, Some("tiny".to_string())));
    }

    /// An agent can edit `ai.yaml`. Pointing a provider at another host
    /// must not carry the person's key there: the key is bound to the URL
    /// it was saved for, until a person re-saves the provider.
    #[tokio::test]
    async fn a_key_goes_only_to_the_url_it_was_saved_for() {
        let (base, _) = mock("/chat/completions", 200, chat_reply()).await;
        let (evil, evil_seen) = mock("/chat/completions", 200, chat_reply()).await;
        let dir = tempfile::tempdir().unwrap();
        let (svc, _db) = service(&base, &dir);
        let provider = |url: &str| ProviderConfig {
            id: "or".into(),
            kind: ProviderKind::Openrouter,
            base_url: Some(url.into()),
        };
        svc.save_provider(provider(&base), Some("sk-2".into()))
            .unwrap();
        svc.complete(Role::Summarize, "t", None, "hi", false)
            .await
            .unwrap();

        // Out-of-band edit, as an agent's shell would make it.
        let mut cfg = AiConfig::load(dir.path()).unwrap();
        cfg.providers[0].base_url = Some(evil.clone());
        cfg.save(dir.path()).unwrap();
        let err = svc
            .complete(Role::Summarize, "t", None, "hi", false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("re-save"), "{err}");
        assert!(
            evil_seen.lock().unwrap().is_empty(),
            "the key must not be sent"
        );

        // A person re-saving the provider (the UI) rebinds the kept key.
        svc.save_provider(provider(&evil), None).unwrap();
        svc.complete(Role::Summarize, "t", None, "hi", false)
            .await
            .unwrap();
        assert_eq!(
            evil_seen.lock().unwrap()[0].1["authorization"],
            "Bearer sk-2"
        );
    }
}
