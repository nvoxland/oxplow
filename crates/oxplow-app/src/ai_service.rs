//! Oxplow's model calls, by role. Resolves a role to a provider and model
//! from the user-global `ai.yaml` (plus any project overrides), fetches the
//! provider's key from the keychain, and records every call in `ai_call`
//! (`v_ai_call`).
//! See `.context/ai-providers.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub use oxplow_ai::capabilities::{CapabilitySnapshotV1, ModelCatalogV1};
use oxplow_ai::client::{
    AiError, CompleteRequest, DecideRequest, ModelProvider, ModelProviders, ProviderInstance,
};
/// Re-exported so the IPC and MCP adapters need only `oxplow-app`.
pub use oxplow_ai::client::{Answer, Completion, Decision, Question};
use oxplow_ai::config::AiConfig;
pub use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_ai::parts::{PartV1, UsageSourceV1};
pub use oxplow_ai::parts::{TypedRequestV1, TypedResponseV1};
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
    pub kind: String,
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
    /// The kinds a provider can be: the registered model providers.
    pub kinds: Vec<ProviderKindInfo>,
}

/// A kind of provider, as the Settings form offers it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKindInfo {
    /// What a provider's `kind:` names.
    pub kind: String,
    pub title: String,
    /// Its API base when a provider names none; `None` when one must.
    pub default_base_url: Option<String>,
}

/// Who a call is for, as its `ai_call` row records it.
#[derive(Debug, Clone, Copy)]
pub struct CallSite<'a> {
    /// What asked (`thread:<id>` for an agent's MCP call,
    /// `collector:<owner>/<id>`, `inferred-decisions`, …).
    pub caller: &'a str,
    /// For a recorded computation (`ai_compute`), the hash of its input.
    pub input_hash: Option<&'a str>,
    /// The extension run that made it (`extension_run.run_id`).
    pub run: Option<&'a str>,
}

impl<'a> CallSite<'a> {
    pub fn new(caller: &'a str) -> Self {
        Self {
            caller,
            input_hash: None,
            run: None,
        }
    }
}

/// What a model is asked, as its call keeps it (`ai_call.request_hash`)
/// and a recorded result is keyed by (`ai_result.request_hash`): the
/// whole prompt, so a changed prompt is another request.
pub enum ModelRequest<'a> {
    Complete {
        system: Option<&'a str>,
        prompt: &'a str,
        json: bool,
    },
    Decide {
        state: &'a str,
        questions: &'a BTreeMap<String, Question>,
    },
    Typed(&'a TypedRequestV1),
}

impl ModelRequest<'_> {
    pub fn body(&self) -> serde_json::Value {
        match self {
            ModelRequest::Complete {
                system,
                prompt,
                json,
            } => serde_json::json!({ "system": system, "prompt": prompt, "json": json }),
            ModelRequest::Decide { state, questions } => {
                serde_json::json!({ "state": state, "questions": questions })
            }
            ModelRequest::Typed(request) => {
                serde_json::to_value(request).expect("request serializes")
            }
        }
    }

    /// Its content hash.
    pub fn hash(&self) -> String {
        oxplow_db::ai_call_store::request_hash(&self.body())
    }
}

/// One call as it is recorded: who asked, what answered, when it started,
/// and what it was asked.
struct Attempt<'a> {
    role: Role,
    site: CallSite<'a>,
    provider: &'a ProviderConfig,
    binding: &'a RoleBinding,
    started: Instant,
    request: &'a ModelRequest<'a>,
}

pub struct AiService {
    /// The model providers the extensions declare, by kind.
    providers: ModelProviders,
    secrets: Arc<dyn SecretStore>,
    calls: Arc<SqliteAiCallStore>,
    /// Where `ai.yaml` lives; `None` when there's no config dir, which
    /// leaves every role unconfigured.
    config_dir: Option<PathBuf>,
    /// The project's role assignments, read on every resolve so edits to
    /// `project.yaml` apply without syncing anything.
    overrides: OverridesSource,
    /// The project's activation boundary; a service built alone has its own,
    /// which nothing has activated.
    activation: Arc<crate::extension_activation::ExtensionActivation>,
    identity: Arc<()>,
}

tokio::task_local! {
    static CONFIG_READERS: Vec<(Arc<()>, AiConfig)>;
}

/// Where project role overrides come from (the live project config).
pub type OverridesSource = Arc<dyn Fn() -> BTreeMap<Role, RoleBinding> + Send + Sync>;

impl AiService {
    pub fn new(
        providers: ModelProviders,
        secrets: Arc<dyn SecretStore>,
        calls: Arc<SqliteAiCallStore>,
        config_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            providers,
            secrets,
            calls,
            config_dir,
            overrides: Arc::new(BTreeMap::new),
            activation: Arc::new(crate::extension_activation::ExtensionActivation::new()),
            identity: Arc::new(()),
        }
    }

    /// Layer `source`'s role assignments (the project's) over `ai.yaml`.
    /// This machine's AI setup: the global `ai.yaml` and the OS keychain
    /// (read only here), recording calls into `calls` and calling through
    /// `providers`. What a check run outside the app uses
    /// (`OXPLOW_LIVE_ANSWERABILITY`).
    pub fn for_this_machine(
        calls: Arc<oxplow_db::SqliteAiCallStore>,
        providers: ModelProviders,
    ) -> Self {
        Self::new(
            providers,
            Arc::new(oxplow_ai::secrets::KeychainSecrets),
            calls,
            oxplow_config::global_config_dir(),
        )
    }

    pub fn with_project_overrides(mut self, source: OverridesSource) -> Self {
        self.overrides = source;
        self
    }

    pub fn with_activation(
        mut self,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Self {
        self.activation = activation;
        self
    }

    pub(crate) async fn read_generation<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, AiServiceError> {
        self.activation
            .read(self.read_configured(future))
            .await
            .map_err(|_| AiServiceError::Config("extension activation is unavailable".into()))?
    }

    async fn read_configured<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, AiServiceError> {
        let mut readers = CONFIG_READERS.try_with(Clone::clone).unwrap_or_default();
        if readers
            .iter()
            .any(|(owner, _)| Arc::ptr_eq(owner, &self.identity))
        {
            return Ok(future.await);
        }
        readers.push((self.identity.clone(), self.config()?));
        Ok(CONFIG_READERS.scope(readers, future).await)
    }

    pub(crate) async fn read_run<F: std::future::Future>(
        &self,
        run: &oxplow_domain::run::RunContext,
        future: F,
    ) -> Result<F::Output, AiServiceError> {
        let run = self.activation.pin_run(run).await.map_err(|_| {
            AiServiceError::Config("extension generation context is unavailable".into())
        })?;
        let pin = run
            .host_generation::<crate::extension_activation::GenerationPin>()
            .expect("pinned AI context");
        self.activation
            .read_pinned(pin, self.read_configured(future))
            .await
            .map_err(|_| {
                AiServiceError::Config("extension generation context is unavailable".into())
            })?
    }

    /// Production metadata ingress; synchronous settings remains a standalone helper.
    pub async fn settings_pinned(&self) -> Result<AiSettings, AiServiceError> {
        self.read_generation(async { self.settings() }).await?
    }

    /// The model providers it calls through.
    pub fn providers(&self) -> &ModelProviders {
        &self.providers
    }

    /// The registered provider `provider`'s kind names, when its `baseUrl`
    /// is given or the kind has a default.
    fn provider_of(
        &self,
        provider: &ProviderConfig,
    ) -> Result<Arc<dyn ModelProvider>, AiServiceError> {
        let p = self
            .providers
            .get(&provider.kind)
            .map_err(|e| AiServiceError::Config(format!("AI provider `{}`: {e}", provider.id)))?;
        if p.default_base_url().is_none()
            && provider
                .base_url
                .as_deref()
                .is_none_or(|u| u.trim().is_empty())
        {
            return Err(AiServiceError::Config(format!(
                "AI provider `{}` is `{}` and needs a baseUrl (e.g. http://localhost:11434/v1)",
                provider.id, provider.kind
            )));
        }
        Ok(p)
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
                    kind: p.kind.clone(),
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
        let kinds = self
            .providers
            .kinds()
            .iter()
            .filter_map(|k| self.providers.get(k).ok())
            .map(|p| ProviderKindInfo {
                kind: p.kind().to_string(),
                title: p.title().to_string(),
                default_base_url: p.default_base_url().map(str::to_string),
            })
            .collect();
        Ok(AiSettings {
            providers,
            roles,
            kinds,
        })
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
        self.provider_of(&provider)?;
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
        self.read_generation(self.test_provider_unpinned(id, model))
            .await?
    }

    async fn test_provider_unpinned(
        &self,
        id: &str,
        model: &str,
    ) -> Result<String, AiServiceError> {
        let provider = self
            .config()?
            .providers
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AiServiceError::Config(format!("no AI provider `{id}`")))?;
        let key = self.key(&provider)?;
        let instance = instance(&provider, key.as_deref());
        Ok(self.provider_of(&provider)?.test(&instance, model).await?)
    }

    /// The effective configuration: global `ai.yaml` plus project overrides.
    pub fn config(&self) -> Result<AiConfig, AiServiceError> {
        if let Some(config) = CONFIG_READERS
            .try_with(|readers| {
                readers
                    .iter()
                    .find(|(owner, _)| Arc::ptr_eq(owner, &self.identity))
                    .map(|(_, config)| config.clone())
            })
            .ok()
            .flatten()
        {
            return Ok(config);
        }
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
        self.read_generation(self.complete_as_unpinned(role, site, system, prompt, json))
            .await?
    }

    async fn complete_as_unpinned(
        &self,
        role: Role,
        site: CallSite<'_>,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<(Completion, Option<i64>), AiServiceError> {
        let (provider, binding, key) = self.prepare(role)?;
        let model_provider = self.provider_of(&provider)?;
        let started = Instant::now();
        let result = model_provider
            .complete(
                &instance(&provider, key.as_deref()),
                &CompleteRequest {
                    model: &binding.model,
                    system,
                    prompt,
                    json,
                },
            )
            .await;
        let request = ModelRequest::Complete {
            system,
            prompt,
            json,
        };
        let outcome = result.as_ref().map(|c| {
            (
                c.input_tokens,
                c.output_tokens,
                serde_json::json!({ "text": c.text }),
            )
        });
        let attempt = Attempt {
            role,
            site,
            provider: &provider,
            binding: &binding,
            started,
            request: &request,
        };
        let call_id = self.record(attempt, outcome).await;
        Ok((result?, call_id))
    }

    /// Complete ordered parts on the configured role. The caller cannot use
    /// this surface to select another model, endpoint or credential.
    pub async fn complete_typed_as(
        &self,
        role: Role,
        site: CallSite<'_>,
        request: &TypedRequestV1,
    ) -> Result<(TypedResponseV1, Option<i64>), AiServiceError> {
        self.read_generation(self.complete_typed_as_unpinned(role, site, request))
            .await?
    }

    async fn complete_typed_as_unpinned(
        &self,
        role: Role,
        site: CallSite<'_>,
        request: &TypedRequestV1,
    ) -> Result<(TypedResponseV1, Option<i64>), AiServiceError> {
        request.validate_version().map_err(AiError::from)?;
        let (provider, binding) = self.resolve(role)?;
        if request.model != binding.model {
            return Err(AiServiceError::Config(
                "typed request must use the role's configured model".into(),
            ));
        }
        // Opaque assets need an authorized host resolver before transport.
        // Never reinterpret them as paths, URLs or inline bytes here.
        if request
            .messages
            .iter()
            .flat_map(|m| &m.parts)
            .any(|p| matches!(p, PartV1::Image { .. }))
        {
            return Err(AiError::UnsupportedFeature {
                provider: provider.id.clone(),
                feature: "imageAssetResolution".into(),
                support: oxplow_ai::capabilities::CapabilitySupport::Unsupported,
            }
            .into());
        }
        let model_provider = self.provider_of(&provider)?;
        let metadata = model_provider
            .discover(&instance(&provider, None), &binding.model)
            .await?;
        metadata.validate_version().map_err(AiError::from)?;
        if metadata.model.as_deref() != Some(&binding.model) {
            return Err(AiServiceError::Config(
                "capability metadata does not identify the role's model".into(),
            ));
        }
        let parts: Vec<_> = request.messages.iter().flat_map(|m| &m.parts).collect();
        let features = [
            (
                "text",
                metadata.text,
                parts.iter().any(|part| matches!(part, PartV1::Text { .. })),
            ),
            (
                "toolCalling",
                metadata.tool_calling,
                !request.tools.is_empty()
                    || parts.iter().any(|part| {
                        matches!(part, PartV1::ToolCall { .. } | PartV1::ToolResult { .. })
                    })
                    || request
                        .messages
                        .iter()
                        .any(|m| m.role == oxplow_ai::parts::MessageRoleV1::Tool),
            ),
        ];
        for (feature, support, requested) in features {
            if requested && support == oxplow_ai::capabilities::CapabilitySupport::Unsupported {
                return Err(AiError::UnsupportedFeature {
                    provider: provider.id.clone(),
                    feature: feature.into(),
                    support,
                }
                .into());
            }
        }
        let key = self.key(&provider)?;
        let started = Instant::now();
        let result = model_provider
            .complete_typed(&instance(&provider, key.as_deref()), request)
            .await
            .and_then(|response| {
                response.validate_version()?;
                if response
                    .parts
                    .iter()
                    .any(|part| matches!(part, PartV1::Image { .. }))
                {
                    return Err(AiError::UnsupportedFeature {
                        provider: provider.id.clone(),
                        feature: "imageAssetResolution".into(),
                        support: oxplow_ai::capabilities::CapabilitySupport::Unsupported,
                    });
                }
                Ok(response)
            });
        let stored_request = ModelRequest::Typed(request);
        let outcome = result.as_ref().map(|response| {
            // The aggregate columns count observed usage only. Optional
            // counts and provenance are preserved in the complete payload.
            let (input, output) = observed_counts(response);
            (
                input,
                output,
                serde_json::to_value(response).expect("response serializes"),
            )
        });
        let call_id = self
            .record(
                Attempt {
                    role,
                    site,
                    provider: &provider,
                    binding: &binding,
                    started,
                    request: &stored_request,
                },
                outcome,
            )
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
        self.read_generation(self.decide_as_unpinned(role, site, state, questions))
            .await?
    }

    async fn decide_as_unpinned(
        &self,
        role: Role,
        site: CallSite<'_>,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<(Decision, Option<i64>), AiServiceError> {
        let (provider, binding, key) = self.prepare(role)?;
        let model_provider = self.provider_of(&provider)?;
        let started = Instant::now();
        let result = model_provider
            .decide(
                &instance(&provider, key.as_deref()),
                &DecideRequest {
                    model: &binding.model,
                    state,
                    questions,
                },
            )
            .await;
        let request = ModelRequest::Decide { state, questions };
        let outcome = result.as_ref().map(|d| {
            (
                d.input_tokens,
                d.output_tokens,
                serde_json::json!({ "answers": d.answers }),
            )
        });
        let attempt = Attempt {
            role,
            site,
            provider: &provider,
            binding: &binding,
            started,
            request: &request,
        };
        let call_id = self.record(attempt, outcome).await;
        Ok((result?, call_id))
    }

    /// The provider and model `role` is bound to now (what a recorded
    /// result is keyed by), without calling it.
    pub fn binding_for(&self, role: Role) -> Result<RoleBinding, AiServiceError> {
        self.config()?
            .resolve(role)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| AiServiceError::NotConfigured(role_name(role)))
    }

    /// Metadata for the role's selected model. Never fetches a key or makes
    /// a completion, and records no paid call.
    pub async fn capabilities(&self, role: Role) -> Result<CapabilitySnapshotV1, AiServiceError> {
        self.read_generation(self.capabilities_unpinned(role))
            .await?
    }

    async fn capabilities_unpinned(
        &self,
        role: Role,
    ) -> Result<CapabilitySnapshotV1, AiServiceError> {
        let (provider, binding) = self.resolve(role)?;
        let metadata = self
            .provider_of(&provider)?
            .discover(&instance(&provider, None), &binding.model)
            .await?;
        metadata.validate_version().map_err(AiError::from)?;
        if metadata.model.as_deref() != Some(&binding.model) {
            return Err(AiServiceError::Config(
                "capability metadata does not identify the role's model".into(),
            ));
        }
        Ok(metadata)
    }

    /// Enumerated metadata from the role's provider, without keychain access.
    pub async fn models(&self, role: Role) -> Result<ModelCatalogV1, AiServiceError> {
        self.read_generation(self.models_unpinned(role)).await?
    }

    async fn models_unpinned(&self, role: Role) -> Result<ModelCatalogV1, AiServiceError> {
        let (provider, _) = self.resolve(role)?;
        let metadata = self
            .provider_of(&provider)?
            .discover_models(&instance(&provider, None))
            .await?;
        metadata.validate().map_err(AiServiceError::Config)?;
        Ok(metadata)
    }

    fn resolve(&self, role: Role) -> Result<(ProviderConfig, RoleBinding), AiServiceError> {
        self.config()?
            .resolve(role)
            .map(|(provider, binding)| (provider.clone(), binding.clone()))
            .ok_or_else(|| AiServiceError::NotConfigured(role_name(role)))
    }

    /// Resolve `role` and fetch its provider's key.
    /// Keys are stored under the provider id; a provider without one (a
    /// local server) is called without auth.
    fn prepare(
        &self,
        role: Role,
    ) -> Result<(ProviderConfig, RoleBinding, Option<String>), AiServiceError> {
        let (provider, binding) = self.resolve(role)?;
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
                 If that's intended, re-save the AI provider in Settings → AI.",
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
        attempt: Attempt<'_>,
        outcome: Result<(i64, i64, serde_json::Value), &AiError>,
    ) -> Option<i64> {
        let Attempt {
            role,
            site,
            provider,
            binding,
            started,
            request,
        } = attempt;
        let (error, (input_tokens, output_tokens, response)) = match outcome {
            Ok((i, o, r)) => (None, (i, o, Some(r))),
            Err(e) => (Some(e), (0, 0, None)),
        };
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
            request: request.body(),
            response,
            run_id: site.run.map(str::to_string),
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

/// Typed payloads retain optional counts and provenance. The numeric
/// aggregates include only provider-observed usage and are zero for missing
/// or non-observed values.
pub(crate) fn observed_counts(response: &TypedResponseV1) -> (i64, i64) {
    if response.usage.source != UsageSourceV1::Observed {
        return (0, 0);
    }
    let count = |value: Option<u64>| value.and_then(|n| i64::try_from(n).ok()).unwrap_or(0);
    (
        count(response.usage.input_tokens),
        count(response.usage.output_tokens),
    )
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

/// `provider` as a call sees it, with its `key`.
fn instance<'a>(provider: &'a ProviderConfig, key: Option<&'a str>) -> ProviderInstance<'a> {
    ProviderInstance {
        id: &provider.id,
        base_url: provider.base_url.as_deref(),
        key,
    }
}

/// Providers prepared from one declared implementation snapshot. Preparing a
/// candidate does not publish it into the live model-provider map.
pub struct PreparedModelProviders {
    providers: Vec<Arc<dyn ModelProvider>>,
}

impl PreparedModelProviders {
    pub fn publish(self, providers: &ModelProviders) {
        providers.set(self.providers);
    }
}

/// Build the scripted model providers named by `declared`. This is a pure
/// staging step: script factories and consent gates are captured, but no
/// provider is published or called.
pub fn prepare_built_ins(
    declared: &[crate::capabilities::Implementation],
    approvals: &Arc<crate::exec_consent::ApprovalStore>,
    project_dir: &std::path::Path,
) -> Result<PreparedModelProviders, BTreeMap<String, String>> {
    let (prepared, errors) = build_built_ins(declared, approvals, project_dir);
    if errors.is_empty() {
        Ok(prepared)
    } else {
        Err(errors)
    }
}

fn build_built_ins(
    declared: &[crate::capabilities::Implementation],
    approvals: &Arc<crate::exec_consent::ApprovalStore>,
    project_dir: &std::path::Path,
) -> (PreparedModelProviders, BTreeMap<String, String>) {
    let mut built: Vec<Arc<dyn ModelProvider>> = Vec::new();
    let mut errors = BTreeMap::new();
    for implementation in declared.iter().filter(|i| i.capability == "ai_provider") {
        if let crate::capabilities::Source::Script {
            entry,
            tree,
            script,
            ..
        } = &implementation.source
        {
            let program = crate::exec_consent::ai_provider_program(
                tree,
                implementation.extension.as_deref().unwrap_or_default(),
                &implementation.id,
                entry,
                implementation
                    .config
                    .get("baseUrl")
                    .and_then(serde_json::Value::as_str),
            );
            let gate = crate::exec_consent::loaded_script_gate(
                approvals.clone(),
                project_dir.to_path_buf(),
                program,
                script.clone(),
            );
            match oxplow_ai_providers::scripted(
                &implementation.id,
                &implementation.title,
                entry,
                script,
                &implementation.config,
                gate,
            ) {
                Ok(provider) => built.push(provider),
                Err(_) => {
                    errors.insert(
                        implementation.id.clone(),
                        "scripted model provider failed to load".to_string(),
                    );
                }
            }
        }
    }
    (PreparedModelProviders { providers: built }, errors)
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

/// The built-in model providers, under foundation's ids, for tests that
/// build an `AiService` without booting.
#[cfg(test)]
pub(crate) fn test_providers() -> ModelProviders {
    // The shipped providers, each a script oxplow-foundation declares —
    // open here: a test of the service isn't one of consent.
    let providers = ModelProviders::default();
    let bundled = crate::extensions::load_extensions(std::path::Path::new("/nonexistent"));
    let built = crate::capabilities::declared_by(&bundled)
        .into_iter()
        .filter(|i| i.capability == "ai_provider")
        .filter_map(|i| match &i.source {
            crate::capabilities::Source::Script { entry, script, .. } => {
                let open: oxplow_ai_providers::Gate = Arc::new(|| Ok(()));
                Some(
                    oxplow_ai_providers::scripted(&i.id, &i.title, entry, script, &i.config, open)
                        .expect("a shipped provider loads"),
                )
            }
            _ => None,
        })
        .collect();
    providers.set(built);
    providers
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai::secrets::MemorySecrets;
    use oxplow_ai_fake::mock;
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
                kind: "openrouter".into(),
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
            AiService::new(test_providers(), secrets, calls, Some(dir.path().into())),
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
        // It keeps what it asked and what came back, by content hash.
        let out = crate::sql_gateway::SqlGateway::new(db.clone())
            .query_sql(
                "SELECT request_hash, response_hash FROM v_ai_call",
                vec![],
                None,
            )
            .await
            .unwrap();
        let row = serde_json::to_value(&out.rows).unwrap()[0].clone();
        let request = ModelRequest::Complete {
            system: None,
            prompt: "hi",
            json: false,
        };
        assert_eq!(row[0], request.hash());
        let response = oxplow_db::event_content_store::read(&db, row[1].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8(response).unwrap(), r#"{"text":"hello"}"#);
    }

    /// A provider's `kind:` is a declared provider; an unknown one is
    /// refused naming the registered, and a kind with no default URL needs
    /// one.
    #[tokio::test]
    async fn an_unknown_provider_kind_is_refused_naming_the_registered() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service("http://127.0.0.1:1", &dir);
        let save = |kind: &str, base_url: Option<&str>| {
            svc.save_provider(
                ProviderConfig {
                    id: "x".into(),
                    kind: kind.into(),
                    base_url: base_url.map(str::to_string),
                },
                None,
            )
        };
        let err = save("openai-compatible", None).unwrap_err().to_string();
        assert!(
            err.contains("no model provider kind `openai-compatible`")
                && err.contains("openai_compatible, openrouter"),
            "{err}"
        );
        let err = save("openai_compatible", None).unwrap_err().to_string();
        assert!(
            err.contains("AI provider `x` is `openai_compatible` and needs a baseUrl"),
            "{err}"
        );
        save("openai_compatible", Some("http://localhost:11434/v1")).unwrap();
        save("anthropic", None).unwrap();
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
                kind: "typesafe".into(),
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
                kind: "typesafe".into(),
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
                Role::Summarize,
                RoleBinding {
                    provider: "or".into(),
                    model: "small".into(),
                },
            )])
        }));
        let st = svc.settings().unwrap();
        let summarize = st.roles.iter().find(|r| r.role == Role::Summarize).unwrap();
        assert!(summarize.overridden);
        assert_eq!(summarize.binding.as_ref().unwrap().model, "small");
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
                kind: "typesafe".into(),
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

    /// An AI provider an extension writes as a script is a kind a person
    /// may configure; its calls go out only once they approved the script
    /// in Settings → Data → Programs, and an edit to it stops them again.
    #[tokio::test]
    async fn a_scripted_provider_calls_once_a_person_approves_it() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let ext = dir.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("providers")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: p\nimplementations:\n  - { capability: ai_provider, id: acme, title: Acme, entry: providers/acme.star }\n",
        )
        .unwrap();
        let script = |reply: &str| {
            format!(
                "def request(x):\n    return {{\"path\": \"/chat/completions\", \"headers\": {{\"authorization\": \"Bearer {{{{key}}}}\"}}, \"body\": {{\"model\": x[\"model\"]}}}}\n\ndef response(x):\n    return {{\"text\": {reply}}}\n"
            )
        };
        std::fs::write(
            ext.join("providers/acme.star"),
            script("x[\"body\"][\"choices\"][0][\"message\"][\"content\"]"),
        )
        .unwrap();
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let (base, seen) = mock("/chat/completions", 200, chat_reply()).await;
        svc.ai
            .save_provider(
                ProviderConfig {
                    id: "a".into(),
                    kind: "acme".into(),
                    base_url: Some(base),
                },
                Some("sk-1".into()),
            )
            .unwrap();
        svc.ai
            .set_role(
                Role::Summarize,
                Some(RoleBinding {
                    provider: "a".into(),
                    model: "m".into(),
                }),
            )
            .unwrap();
        let call = || svc.ai.complete(Role::Summarize, "t", None, "hi", false);
        let err = call().await.unwrap_err().to_string();
        assert!(err.contains("Settings → Data → Programs"), "{err}");
        assert!(seen.lock().unwrap().is_empty(), "nothing was sent");

        let config = svc.config.read().unwrap().clone();
        let extensions = svc.extension_catalog.get(dir.path());
        let listed = crate::exec_consent::list(&svc.approvals, dir.path(), &config, &extensions)
            .into_iter()
            .find(|p| p.kind == crate::exec_consent::ProgramKind::AiProvider)
            .expect("it is listed to approve");
        assert_eq!(listed.name, "acme/acme");
        crate::exec_consent::approve_program(
            &svc.approvals,
            dir.path(),
            &config,
            &extensions,
            crate::exec_consent::ProgramKind::AiProvider,
            &listed.name,
            listed.version.as_deref().unwrap(),
        )
        .unwrap();
        assert_eq!(call().await.unwrap().text, "hello");
        assert_eq!(seen.lock().unwrap()[0].1["authorization"], "Bearer sk-1");

        std::fs::write(ext.join("providers/acme.star"), script("\"changed\"")).unwrap();
        let err = call().await.unwrap_err().to_string();
        assert!(
            err.contains("Settings → Data → Programs"),
            "an edit asks again: {err}"
        );
    }

    /// The shipped providers are scripts (oxplow-foundation's
    /// `providers/*.star`); each speaks its API as the built-in it replaced
    /// did. `shipped(kind)` is the registered script.
    fn shipped(kind: &str) -> Arc<dyn ModelProvider> {
        test_providers().get(kind).expect("a shipped provider")
    }

    fn instance<'a>(
        base: &'a str,
        key: Option<&'a str>,
    ) -> oxplow_ai::client::ProviderInstance<'a> {
        oxplow_ai::client::ProviderInstance {
            id: "p",
            base_url: Some(base),
            key,
        }
    }

    fn ask(
        model: &'static str,
        system: Option<&'static str>,
        json: bool,
    ) -> CompleteRequest<'static> {
        CompleteRequest {
            model,
            system,
            prompt: "hello",
            json,
        }
    }

    fn questions() -> BTreeMap<String, Question> {
        BTreeMap::from([
            (
                "risky".to_string(),
                Question::Noul {
                    instructions: "Is this change risky?".into(),
                },
            ),
            (
                "area".to_string(),
                Question::Choice {
                    instructions: "Which area?".into(),
                    options: vec!["ui".into(), "db".into()],
                },
            ),
        ])
    }

    /// Every AI provider oxplow ships is a script a person approves:
    /// none is a built-in, and each loads.
    #[test]
    fn every_shipped_ai_provider_is_a_script() {
        let bundled = crate::extensions::load_extensions(std::path::Path::new("/nonexistent"));
        let providers: Vec<_> = crate::capabilities::declared_by(&bundled)
            .into_iter()
            .filter(|i| i.capability == "ai_provider")
            .collect();
        assert_eq!(
            providers.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec![
                "anthropic",
                "openai",
                "openai_compatible",
                "openrouter",
                "typesafe"
            ]
        );
        assert!(providers
            .iter()
            .all(|i| matches!(i.source, crate::capabilities::Source::Script { .. })));
        assert_eq!(test_providers().kinds().len(), providers.len());
    }

    #[tokio::test]
    async fn the_anthropic_script_speaks_messages() {
        let (base, seen) = mock(
            "/v1/messages",
            200,
            json!({"content": [{"type": "text", "text": "hi"}], "usage": {"input_tokens": 7, "output_tokens": 2}}),
        )
        .await;
        let c = shipped("anthropic")
            .complete(
                &instance(&base, Some("k1")),
                &ask("claude-x", Some("be brief"), false),
            )
            .await
            .unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("hi", 7, 2)
        );
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["x-api-key"], "k1");
        assert!(headers.contains_key("anthropic-version"));
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["messages"][0]["content"], "hello");
        assert_eq!(
            shipped("anthropic").default_base_url(),
            Some("https://api.anthropic.com")
        );
    }

    /// An OpenAI-compatible API: a local one takes no key and gets no auth
    /// header; JSON mode asks for an object; a typed question is asked as
    /// a chat; a refused key and a rate limit read as such.
    #[tokio::test]
    async fn the_openai_compatible_script_speaks_chat() {
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": "{\"a\":1}"}}], "usage": {"prompt_tokens": 5, "completion_tokens": 3}}),
        )
        .await;
        let local = shipped("openai_compatible");
        assert_eq!(
            local.default_base_url(),
            None,
            "every instance names its URL"
        );
        let c = local
            .complete(&instance(&base, None), &ask("qwen3", None, true))
            .await
            .unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("{\"a\":1}", 5, 3)
        );
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert!(
            !headers.contains_key("authorization"),
            "a local server gets no auth header"
        );
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(
            shipped("openai").default_base_url(),
            Some("https://api.openai.com/v1")
        );

        let reply = json!({"answers": {"risky": {"type": "noul", "probability": 0.3}, "area": {"type": "choice", "choice": "ui", "probabilities": {"ui": 0.7, "db": 0.3}}}});
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 50, "completion_tokens": 20}}),
        )
        .await;
        let qs = questions();
        let d = local
            .decide(
                &instance(&base, None),
                &oxplow_ai::client::DecideRequest {
                    model: "qwen3",
                    state: "diff…",
                    questions: &qs,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.3 });
        assert_eq!(d.input_tokens, 50);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("Is this change risky?"));

        let (base, _) = mock("/chat/completions", 401, json!({"error": "bad key"})).await;
        assert!(matches!(
            local
                .complete(&instance(&base, Some("x")), &ask("m", None, false))
                .await,
            Err(AiError::Auth { .. })
        ));
        let (base, _) = mock("/chat/completions", 429, json!({})).await;
        assert!(matches!(
            local
                .complete(&instance(&base, Some("x")), &ask("m", None, false))
                .await,
            Err(AiError::RateLimited { .. })
        ));
    }

    /// OpenRouter chats like OpenAI; a Jev model answers typed questions
    /// natively on `/systemone`.
    #[tokio::test]
    async fn the_openrouter_script_decides_natively_for_jev() {
        let (base, seen) = mock(
            "/systemone",
            200,
            json!({"answers": {"risky": {"type": "noul", "noul": 0.6}, "area": {"type": "choice", "choice": "db", "probabilities": {"ui": 0.1, "db": 0.9}}}, "usage": {"input_tokens": 3, "output_tokens": 0}}),
        )
        .await;
        let qs = questions();
        let d = shipped("openrouter")
            .decide(
                &instance(&base, Some("or")),
                &oxplow_ai::client::DecideRequest {
                    model: "typesafe/jev",
                    state: "diff…",
                    questions: &qs,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.6 });
        assert!(matches!(&d.answers["area"], Answer::Choice { choice, .. } if choice == "db"));
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer or");
        assert_eq!(
            body["questions"]["area"]["criteria"]["db"],
            serde_json::Value::Null
        );
        assert_eq!(
            shipped("openrouter").default_base_url(),
            Some("https://openrouter.ai/api/v1")
        );
    }

    /// TypeSafe only answers typed questions (Jev's `/v1/systemone`); it
    /// can't chat, so its connection check asks one yes/no question.
    #[tokio::test]
    async fn the_typesafe_script_only_decides() {
        let (base, seen) = mock(
            "/v1/systemone",
            200,
            json!({"model": "jev-1.13.0", "answers": {"risky": {"type": "noul", "noul": 0.8}, "area": {"type": "choice", "choice": "db", "probabilities": {"ui": 0.1, "db": 0.9}, "confidence": 0.8}, "ok": {"type": "noul", "noul": 0.9}}, "usage": {"input_tokens": 1000000, "output_tokens": 0}}),
        )
        .await;
        let typesafe = shipped("typesafe");
        let qs = questions();
        let d = typesafe
            .decide(
                &instance(&base, Some("tk")),
                &oxplow_ai::client::DecideRequest {
                    model: "jev-latest",
                    state: "diff…",
                    questions: &qs,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.8 });
        assert!(matches!(&d.answers["area"], Answer::Choice { choice, .. } if choice == "db"));
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer tk");
        assert_eq!(body["state"], "diff…");
        assert_eq!(body["questions"]["risky"]["type"], "noul");
        assert!(typesafe
            .complete(&instance(&base, Some("tk")), &ask("jev", None, false))
            .await
            .is_err());
        assert_eq!(
            typesafe.test(&instance(&base, None), "jev").await.unwrap(),
            "Answered (yes: 90%)"
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
                format!("ai:\n  roles:\n    summarize: {{ provider: or, model: {model} }}\n"),
            )
            .unwrap();
        };
        yaml("small");
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        svc.ai
            .save_provider(
                ProviderConfig {
                    id: "or".into(),
                    kind: "openrouter".into(),
                    base_url: None,
                },
                None,
            )
            .unwrap();
        let summarize = |svc: &crate::Services| {
            let st = svc.ai.settings().unwrap();
            let r = st
                .roles
                .into_iter()
                .find(|r| r.role == Role::Summarize)
                .unwrap();
            (r.overridden, r.binding.map(|b| b.model))
        };
        assert_eq!(summarize(&svc), (true, Some("small".to_string())));
        yaml("tiny");
        svc.reload_config_from_disk().unwrap();
        crate::test_fixtures::activate_with_problems(&svc).await;
        assert_eq!(summarize(&svc), (true, Some("tiny".to_string())));
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
            kind: "openrouter".into(),
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
        assert!(err.to_string().contains("re-save the AI provider"), "{err}");
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

/// The typed surface end to end: discovery from declared metadata, mixed
/// text/tool parts through a provider, usage provenance, refused
/// modalities, and the recorded provenance. No real network or keychain.
#[cfg(test)]
mod typed_tests {
    use super::*;
    use oxplow_ai::capabilities::CapabilitySupport;
    use oxplow_ai::parts::{FinishReasonV1, MessageRoleV1, MessageV1, ToolDefinitionV1};
    use oxplow_ai::secrets::{MemorySecrets, SecretError};
    use oxplow_ai_fake::{mock, Seen};
    use oxplow_db::Database;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts keychain reads, so a test can say none happened.
    #[derive(Default)]
    struct WatchedSecrets {
        inner: MemorySecrets,
        gets: AtomicUsize,
    }

    impl SecretStore for WatchedSecrets {
        fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.inner.get(name)
        }
        fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
            self.inner.set(name, value)
        }
        fn delete(&self, name: &str) -> Result<(), SecretError> {
            self.inner.delete(name)
        }
    }

    /// Hands the mock's body back as the V1 response, whatever it says.
    const PASS_THROUGH: &str = r#"
def request(x):
    return {"path": "/typed", "headers": {"authorization": "Bearer {{key}}"}, "body": x["request"]}

def response(x):
    return x["body"]
"#;

    fn snapshot(model: &str, tools: &str, image: &str) -> Value {
        json!({ "schemaVersion": 1, "model": model, "modelKnown": true,
                "text": "supported", "image": image, "toolCalling": tools,
                "streaming": "unknown",
                "contextTokens": { "value": 8000, "unit": "tokens" }, "outputTokens": null })
    }

    struct Fixture {
        svc: AiService,
        db: Database,
        secrets: Arc<WatchedSecrets>,
        seen: Seen,
    }

    /// A service whose `summarize` role is model `m` of provider `p`
    /// (kind `fake`, the pass-through script with `models` declared, or
    /// the shipped `openai_compatible` when `shipped`), answering `reply`.
    async fn fixture(kind: &str, models: Option<Value>, reply: Value) -> Fixture {
        let path = if kind == "fake" {
            "/typed"
        } else {
            "/chat/completions"
        };
        let (base, seen) = mock(path, 200, reply).await;
        let mut config = json!({ "ops": ["complete", "completeTyped"] });
        if let Some(models) = models {
            config["models"] = models;
        }
        let open: oxplow_ai_providers::Gate = Arc::new(|| Ok(()));
        let fake =
            oxplow_ai_providers::scripted("fake", "Fake", "f.star", PASS_THROUGH, &config, open)
                .unwrap();
        let providers = ModelProviders::default();
        providers.set(vec![
            fake,
            test_providers().get("openai_compatible").unwrap(),
        ]);
        let dir = tempfile::tempdir().unwrap();
        AiConfig {
            providers: vec![ProviderConfig {
                id: "p".into(),
                kind: kind.into(),
                base_url: Some(base.clone()),
            }],
            roles: BTreeMap::from([(
                Role::Summarize,
                RoleBinding {
                    provider: "p".into(),
                    model: "m".into(),
                },
            )]),
        }
        .save(dir.path())
        .unwrap();
        let secrets = Arc::new(WatchedSecrets::default());
        let bound = StoredKey {
            key: "sk-typed-key".into(),
            endpoint: base,
        };
        secrets.set("p", &bound.encode()).unwrap();
        let db = Database::in_memory();
        let calls = Arc::new(SqliteAiCallStore::new(db.clone()));
        let svc = AiService::new(providers, secrets.clone(), calls, Some(dir.path().into()));
        // The config dir must outlive the service.
        std::mem::forget(dir);
        Fixture {
            svc,
            db,
            secrets,
            seen,
        }
    }

    fn typed_reply(source: &str) -> Value {
        json!({ "schemaVersion": 1, "finishReason": "toolCalls",
            "parts": [
                { "type": "text", "text": "first" },
                { "type": "toolCall", "id": "c1", "name": "grep", "arguments": { "q": "x" } },
                { "type": "text", "text": "last" } ],
            "usage": { "inputTokens": 11, "outputTokens": 6, "source": source } })
    }

    fn mixed_request() -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            tools: vec![ToolDefinitionV1 {
                name: "grep".into(),
                description: Some("search".into()),
                parameters: json!({ "type": "object" }),
            }],
            messages: vec![
                MessageV1 {
                    role: MessageRoleV1::User,
                    parts: vec![PartV1::Text {
                        text: "find x".into(),
                    }],
                },
                MessageV1 {
                    role: MessageRoleV1::Assistant,
                    parts: vec![PartV1::ToolCall {
                        id: "c0".into(),
                        name: "grep".into(),
                        arguments: json!({ "q": "y" }),
                    }],
                },
                MessageV1 {
                    role: MessageRoleV1::Tool,
                    parts: vec![PartV1::ToolResult {
                        call_id: "c0".into(),
                        result: json!(["hit"]),
                    }],
                },
            ],
        }
    }

    fn text_request(text: &str) -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            tools: Vec::new(),
            messages: vec![MessageV1 {
                role: MessageRoleV1::User,
                parts: vec![PartV1::Text { text: text.into() }],
            }],
        }
    }

    fn image_request() -> TypedRequestV1 {
        let mut req = text_request("see");
        req.messages[0].parts.push(PartV1::Image {
            asset_ref: "asset:1".into(),
            media_type: "image/png".into(),
        });
        req
    }

    async fn call_rows(db: &Database) -> Value {
        let out = crate::sql_gateway::SqlGateway::new(db.clone())
            .query_sql(
                "SELECT ok, input_tokens, output_tokens, request_hash, response_hash FROM v_ai_call ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&out.rows).unwrap()
    }

    #[tokio::test]
    async fn discovery_returns_declared_metadata_without_network_or_keychain() {
        let models = json!([
            snapshot("m", "supported", "unknown"),
            snapshot("other", "unsupported", "supported")
        ]);
        let fx = fixture("fake", Some(models), typed_reply("observed")).await;
        let snap = fx.svc.capabilities(Role::Summarize).await.unwrap();
        assert_eq!(snap.model.as_deref(), Some("m"));
        assert_eq!(snap.tool_calling, CapabilitySupport::Supported);
        assert_eq!(snap.image, CapabilitySupport::Unknown);
        assert_eq!(snap.context_tokens.as_ref().map(|l| l.value), Some(8000));
        assert_eq!(
            snap.output_tokens, None,
            "an undeclared limit stays unknown"
        );
        let catalog = fx.svc.models(Role::Summarize).await.unwrap();
        assert!(catalog.catalog_known);
        assert_eq!(catalog.models.len(), 2);

        assert_eq!(
            fx.secrets.gets.load(Ordering::SeqCst),
            0,
            "no keychain read"
        );
        assert!(fx.seen.lock().unwrap().is_empty(), "no network call");
        assert_eq!(call_rows(&fx.db).await, json!([]), "nothing recorded");
    }

    /// A provider that declares nothing, or doesn't list the model, is
    /// unknown across the board, with no guess from the model's name.
    #[tokio::test]
    async fn undeclared_metadata_stays_unknown() {
        let fx = fixture("openai_compatible", None, json!({})).await;
        let snap = fx.svc.capabilities(Role::Summarize).await.unwrap();
        assert_eq!(
            snap,
            oxplow_ai::capabilities::CapabilitySnapshotV1::unknown("m")
        );
        assert!(!fx.svc.models(Role::Summarize).await.unwrap().catalog_known);

        let listing_others = json!([snapshot("other", "supported", "supported")]);
        let fx = fixture("fake", Some(listing_others), json!({})).await;
        let snap = fx.svc.capabilities(Role::Summarize).await.unwrap();
        assert!(!snap.model_known);
        assert_eq!(snap.image, CapabilitySupport::Unknown);
        assert_eq!(fx.secrets.gets.load(Ordering::SeqCst), 0);
        assert!(fx.seen.lock().unwrap().is_empty());
    }

    /// Mixed text and tool parts return in the provider's order with the
    /// finish category; observed usage fills the aggregate columns.
    #[tokio::test]
    async fn a_mixed_typed_call_returns_ordered_parts_and_observed_usage() {
        let fx = fixture("fake", None, typed_reply("observed")).await;
        let request = mixed_request();
        let (response, id) = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("test"), &request)
            .await
            .unwrap();
        assert!(id.is_some());
        assert!(matches!(response.parts.as_slice(),
            [PartV1::Text { text: a }, PartV1::ToolCall { id, name, .. }, PartV1::Text { text: b }]
            if a == "first" && id == "c1" && name == "grep" && b == "last"));
        assert_eq!(response.finish_reason, FinishReasonV1::ToolCalls);
        assert_eq!(response.usage.source, UsageSourceV1::Observed);

        // The provider got the whole ordered request and the stored key.
        let (_, headers, body) = fx.seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer sk-typed-key");
        assert_eq!(body, serde_json::to_value(&request).unwrap());

        // The call keeps its request hash and the exact response, and the
        // aggregate columns count the observed usage.
        let rows = call_rows(&fx.db).await;
        assert_eq!(rows[0][0], 1);
        assert_eq!(
            (rows[0][1].clone(), rows[0][2].clone()),
            (json!(11), json!(6))
        );
        assert_eq!(rows[0][3], ModelRequest::Typed(&request).hash());
        let stored = oxplow_db::event_content_store::read(&fx.db, rows[0][4].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        let stored: TypedResponseV1 = serde_json::from_slice(&stored).unwrap();
        assert_eq!(stored, response);
    }

    /// Estimated usage is kept as estimated in the payload and never
    /// leaks into the observed aggregates.
    #[tokio::test]
    async fn estimated_usage_is_not_counted_as_observed() {
        let fx = fixture("fake", None, typed_reply("estimated")).await;
        let (response, _) = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("test"), &mixed_request())
            .await
            .unwrap();
        assert_eq!(response.usage.source, UsageSourceV1::Estimated);
        assert_eq!(response.usage.input_tokens, Some(11));
        assert_eq!(observed_counts(&response), (0, 0));
        let rows = call_rows(&fx.db).await;
        assert_eq!(
            (rows[0][1].clone(), rows[0][2].clone()),
            (json!(0), json!(0))
        );
        let stored = oxplow_db::event_content_store::read(&fx.db, rows[0][4].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        let stored: Value = serde_json::from_slice(&stored).unwrap();
        assert_eq!(stored["usage"]["source"], "estimated");
        assert_eq!(stored["usage"]["inputTokens"], 11);
    }

    /// An image is refused at the app boundary, naming the feature: no
    /// key read, no request, no call row.
    #[tokio::test]
    async fn an_image_part_is_refused_before_anything_happens() {
        let fx = fixture("fake", None, typed_reply("observed")).await;
        let err = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("test"), &image_request())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AiServiceError::Call(AiError::UnsupportedFeature { feature, support, .. })
                if feature == "imageAssetResolution" && *support == CapabilitySupport::Unsupported),
            "{err}"
        );
        assert_eq!(fx.secrets.gets.load(Ordering::SeqCst), 0);
        assert!(fx.seen.lock().unwrap().is_empty());
        assert_eq!(call_rows(&fx.db).await, json!([]));
    }

    /// A modality the model's declared metadata rules out fails before the
    /// key is fetched or a request is sent; an unknown one is attempted.
    #[tokio::test]
    async fn a_declared_unsupported_tool_call_fails_clearly() {
        let models = json!([snapshot("m", "unsupported", "unknown")]);
        let fx = fixture("fake", Some(models), typed_reply("observed")).await;
        let err = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("test"), &mixed_request())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AiServiceError::Call(AiError::UnsupportedFeature { feature, support, .. })
                if feature == "toolCalling" && *support == CapabilitySupport::Unsupported),
            "{err}"
        );
        assert_eq!(fx.secrets.gets.load(Ordering::SeqCst), 0);
        assert!(fx.seen.lock().unwrap().is_empty());
        // Plain text is still fine for that model.
        fx.svc
            .complete_typed_as(Role::Summarize, CallSite::new("test"), &text_request("hi"))
            .await
            .unwrap();
        assert_eq!(fx.seen.lock().unwrap().len(), 1);
    }

    /// A request for another model, or another schema version, is refused
    /// instead of reaching the provider.
    #[tokio::test]
    async fn a_request_must_name_the_roles_model_and_a_known_version() {
        let fx = fixture("fake", None, typed_reply("observed")).await;
        let other_model = TypedRequestV1 {
            model: "other".into(),
            ..text_request("hi")
        };
        let err = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("t"), &other_model)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("role's configured model"), "{err}");
        let stale = TypedRequestV1 {
            schema_version: 2,
            ..text_request("hi")
        };
        let err = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("t"), &stale)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("schemaVersion 2"), "{err}");
        assert!(fx.seen.lock().unwrap().is_empty());
    }

    /// A provider that answers with an image part is refused, not trimmed.
    #[tokio::test]
    async fn an_image_in_a_response_is_refused() {
        let reply = json!({ "schemaVersion": 1, "finishReason": "stop",
            "parts": [{ "type": "image", "assetRef": "a", "mediaType": "image/png" }],
            "usage": { "inputTokens": null, "outputTokens": null, "source": "unknown" } });
        let fx = fixture("fake", None, reply).await;
        let err = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("t"), &text_request("hi"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("imageAssetResolution"), "{err}");
        // The failed attempt is still recorded.
        assert_eq!(call_rows(&fx.db).await[0][0], 0);
    }

    /// The text helpers and the typed call share one provider and one
    /// record; a typed text call over a text-only provider keeps the
    /// legacy usage as unknown-provenance counts.
    #[tokio::test]
    async fn text_helpers_and_the_typed_call_share_the_path() {
        let reply = json!({"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "hello"}}],
                           "usage": {"prompt_tokens": 10, "completion_tokens": 5}});
        let fx = fixture("openai_compatible", None, reply).await;
        let c = fx
            .svc
            .complete(Role::Summarize, "core", None, "hi", false)
            .await
            .unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("hello", 10, 5)
        );
        let qs = BTreeMap::from([(
            "ok".to_string(),
            oxplow_ai::client::Question::Noul {
                instructions: "ok?".into(),
            },
        )]);
        // Not a JSON answer, so decide reports the shape problem, but it
        // went through the same provider.
        assert!(fx
            .svc
            .decide(Role::Summarize, "core", "s", &qs)
            .await
            .is_err());

        let (typed, _) = fx
            .svc
            .complete_typed_as(Role::Summarize, CallSite::new("core"), &text_request("hi"))
            .await
            .unwrap();
        assert_eq!(
            typed.parts,
            vec![PartV1::Text {
                text: "hello".into()
            }]
        );
        assert_eq!(typed.finish_reason, FinishReasonV1::Stop);
        assert_eq!(
            (
                typed.usage.input_tokens,
                typed.usage.output_tokens,
                typed.usage.source
            ),
            (Some(10), Some(5), UsageSourceV1::Observed)
        );
        let rows = call_rows(&fx.db).await;
        assert_eq!(rows.as_array().unwrap().len(), 3);
        assert_eq!(
            rows[0][3],
            ModelRequest::Complete {
                system: None,
                prompt: "hi",
                json: false
            }
            .hash()
        );
        assert_eq!(rows[2][3], ModelRequest::Typed(&text_request("hi")).hash());
        assert_eq!(
            (rows[2][1].clone(), rows[2][2].clone()),
            (json!(10), json!(5))
        );
    }
}
