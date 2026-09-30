//! The provider instances (`Services.providers`): one long-lived process
//! per enabled instance, its health, and restart with backoff.
//!
//! An instance is `<extension>/<provider id>`, configured in the
//! project's `extensionInstances` (`{ enabled, config }`, a person's key).
//! [`ProviderRegistry::reconcile`] makes the running set match it — at
//! boot and on every config change. Starting an instance checks consent,
//! spawns, handshakes and `check`s its config; only then are its declared
//! commands registered on the bus as `<id>.<name>` (`Atomicity::External`)
//! and its capability's provider (`ExternalWorkItems`) in the capability's
//! registry. Every restart goes through consent again.
//!
//! Health is per machine: [`InstanceHealth`]. A start or call that fails
//! (not a refused input) counts; [`FAILURES_TO_DISABLE`] in a row stop the
//! instance, log `provider.disabled@1 { instance, reason }` and keep it off
//! — across restarts, since the log says so — until a person runs
//! `provider.enable`, which logs `provider.enabled@1`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Weak};
use std::time::{Duration, Instant};

use oxplow_ai::secrets::SecretStore;
use oxplow_db::{Database, SqliteEventLogStore};
use oxplow_domain::events::schema::{
    EventType as _, ProviderDisabled, ProviderDisabledV1, ProviderEnabled, ProviderEnabledV1,
    WorkItemRecorded,
};
use oxplow_domain::work_items::WorkItemsRegistry;
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, DomainError,
    Envelope, Invokers, Lifecycle,
};
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, EventDraft, Handle, InitializeResult, InvokeParams,
    InvokeResult,
};
use oxplow_provider_protocol::ProtocolError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::host::{self, Connection, HostError, Launch};
use super::spec::{self, ProviderSpec};
use crate::commands::{Command, CommandBus, Handler, HandlerOutput};
use crate::exec_consent::ApprovalStore;
use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::Extension;

/// Failed starts or calls in a row that stop an instance.
pub const FAILURES_TO_DISABLE: u32 = 3;
/// The longest a failed instance waits before its next start.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The errors an instance's health keeps.
const ERRORS_KEPT: usize = 5;
/// The command a person runs to (re-)enable an instance.
pub const ENABLE: &str = "provider.enable";

/// What the registry needs from the host.
#[derive(Clone)]
pub struct HostDeps {
    /// The primary worktree: where providers run from.
    pub project_dir: PathBuf,
    /// The project's key (credentials are scoped by it).
    pub project: String,
    pub approvals: Arc<ApprovalStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub config: Arc<RwLock<oxplow_config::OxplowConfig>>,
    pub catalog: Arc<ExtensionCatalog>,
    pub db: Database,
    pub log: SqliteEventLogStore,
    pub host_env: host::HostEnv,
    /// The first restart's wait; each failure doubles it, up to
    /// [`MAX_BACKOFF`].
    pub backoff: Duration,
}

/// A config problem `check` reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ConfigProblem {
    /// A JSON pointer into the config (`/team`), `""` for the whole.
    pub path: String,
    pub message: String,
}

/// Where an instance stands on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstanceState {
    /// Not enabled in the project's config.
    Off,
    /// Configured, but no enabled extension declares it.
    Missing,
    /// Enabled, but this machine hasn't approved this version of it.
    Unapproved,
    /// Enabled, but its `check` found problems with its config.
    Unconfigured {
        problems: Vec<ConfigProblem>,
    },
    /// Being checked or started.
    Checking,
    Ready,
    /// Its last start or call failed; it restarts with backoff.
    Failing {
        errors: Vec<String>,
    },
    /// Stopped after repeated failures (or a handshake that didn't match
    /// its approved declarations), until a person enables it again.
    Disabled {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceHealth {
    pub state: InstanceState,
    pub consecutive_failures: u32,
    /// RFC 3339.
    pub last_ok_at: Option<String>,
    /// A moving average of its successful calls.
    pub mean_invoke_ms: Option<f64>,
}

impl InstanceHealth {
    fn new(state: InstanceState) -> Self {
        Self {
            state,
            consecutive_failures: 0,
            last_ok_at: None,
            mean_invoke_ms: None,
        }
    }
}

/// An instance as Settings → Integrations shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstanceView {
    /// `<extension>/<provider id>`.
    pub instance: String,
    pub extension: String,
    pub provider: String,
    pub capability: String,
    /// The project's config enables it.
    pub enabled: bool,
    #[specta(type = oxplow_domain::Json)]
    pub config: Value,
    /// JSON Schema of `config`, from its declarations.
    #[specta(type = oxplow_domain::Json)]
    pub config_schema: Value,
    /// This machine approved it as it is now.
    pub approved: bool,
    /// Each credential it declares and whether this machine has a value.
    pub credentials: Vec<crate::source_runner::CredentialStatus>,
    pub health: InstanceHealth,
}

/// A started process and the handle its `check` returned.
struct Live {
    conn: Connection,
    handle: Handle,
}

/// One enabled instance.
pub struct Instance {
    /// `<extension>/<id>`.
    pub name: String,
    pub ext: Extension,
    pub spec: ProviderSpec,
    pub config: Value,
    /// The approved declarations it was enabled with.
    pub declared: InitializeResult,
    deps: HostDeps,
    registry: Weak<ProviderRegistry>,
    live: tokio::sync::Mutex<Option<Live>>,
    not_before: parking_lot::Mutex<Option<Instant>>,
}

impl Instance {
    fn ext_dir(&self) -> PathBuf {
        host::ext_dir(&self.deps.project_dir, &self.ext)
    }

    /// Consent, spawn, handshake, check.
    async fn start(&self) -> Result<Live, HostError> {
        if !crate::exec_consent::may_run_provider(
            &self.deps.approvals,
            &self.deps.project_dir,
            &self.ext,
            &self.spec,
        ) {
            return Err(HostError::Unapproved(self.name.clone()));
        }
        let dir = self.ext_dir();
        let declared = spec::read_declarations(&self.spec, &|rel| {
            std::fs::read_to_string(dir.join(rel)).ok()
        })
        .map_err(|message| HostError::Failed {
            name: self.name.clone(),
            message,
        })?;
        if declared != self.declared {
            return Err(HostError::DeclarationsChanged {
                name: self.name.clone(),
                detail: "its declarations file changed since it was enabled".into(),
            });
        }
        let mut credentials = BTreeMap::new();
        for name in &self.spec.credentials {
            let account =
                crate::source_runner::credential_account(&self.deps.project, &self.ext.name, name);
            match self.deps.secrets.get(&account) {
                Ok(Some(v)) => {
                    credentials.insert(name.clone(), v);
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(HostError::Failed {
                        name: self.name.clone(),
                        message: format!("credential `{name}`: {e}"),
                    })
                }
            }
        }
        let names: Vec<String> = credentials.keys().cloned().collect();
        let conn = host::connect(&Launch {
            name: self.name.clone(),
            ext_dir: dir,
            spec: self.spec.clone(),
            declared,
            credentials,
            host_env: self.deps.host_env.clone(),
        })
        .await?;
        let checked: CheckResult = conn
            .peer
            .call(
                method::CHECK,
                &CheckParams {
                    config: self.config.clone(),
                    credentials: names,
                },
            )
            .await
            .map_err(|e| HostError::Failed {
                name: self.name.clone(),
                message: format!("check: {e}"),
            })?;
        match checked.handle {
            Some(handle) if checked.problems.is_empty() => Ok(Live { conn, handle }),
            _ => Err(HostError::Unconfigured {
                name: self.name.clone(),
                problems: checked.problems,
            }),
        }
    }

    /// The running process's peer and handle, starting it when it isn't
    /// (unless it's backing off).
    async fn connection(&self) -> Result<(oxplow_provider_protocol::Peer, Handle), CommandError> {
        let mut live = self.live.lock().await;
        if live.as_ref().is_none_or(|l| l.conn.peer.is_closed()) {
            *live = None;
            if let Some(wait) = self
                .not_before
                .lock()
                .and_then(|t| t.checked_duration_since(Instant::now()))
            {
                return Err(CommandError::Failed {
                    message: format!(
                        "provider `{}` failed to start; trying again in {}s",
                        self.name,
                        wait.as_secs() + 1
                    ),
                });
            }
            let started = self.start().await;
            let registry = self.registry.upgrade();
            match started {
                Ok(l) => {
                    *self.not_before.lock() = None;
                    *live = Some(l);
                }
                Err(e) => {
                    let message = e.to_string();
                    drop(live);
                    if let Some(r) = registry {
                        r.start_failed(&self.name, e).await;
                    }
                    return Err(CommandError::Failed { message });
                }
            }
        }
        let l = live.as_ref().expect("started above");
        Ok((l.conn.peer.clone(), l.handle.clone()))
    }

    /// Run one of its declared commands.
    pub async fn invoke(&self, command: &str, input: Value) -> Result<InvokeResult, CommandError> {
        let (peer, handle) = self.connection().await?;
        let started = Instant::now();
        let result = peer
            .call::<_, InvokeResult>(
                method::INVOKE,
                &InvokeParams {
                    handle,
                    command: command.into(),
                    input,
                },
            )
            .await;
        if peer.is_closed() {
            // It died under the call: the next one restarts it.
            self.live.lock().await.take();
        }
        let registry = self.registry.upgrade();
        match result {
            Ok(out) => {
                if let Some(r) = registry {
                    r.call_succeeded(&self.name, started.elapsed());
                }
                Ok(out)
            }
            Err(e) => {
                // A refused input or a cancel is the caller's, not a failure.
                let counts = !matches!(
                    e,
                    ProtocolError::InvalidInput { .. } | ProtocolError::Cancelled
                );
                let err = self.command_error(e);
                if let (true, Some(r)) = (counts, registry) {
                    r.failed(&self.name, err.to_string()).await;
                }
                Err(err)
            }
        }
    }

    fn command_error(&self, e: ProtocolError) -> CommandError {
        match e {
            ProtocolError::InvalidInput { field, message } => CommandError::Invalid {
                field: Some(field),
                message: format!("{}: {message}", self.spec.id),
            },
            other => CommandError::Failed {
                message: format!("provider `{}`: {other}", self.name),
            },
        }
    }

    /// A returned event as the envelope the bus logs: a type it declared,
    /// and (for a work-item record) an item of its own.
    fn envelope(&self, actor: &Actor, draft: EventDraft) -> Result<Envelope, CommandError> {
        let failed = |message: String| CommandError::Failed { message };
        if !self
            .declared
            .event_types
            .iter()
            .any(|t| t.event_type == draft.event_type && t.v == draft.v)
        {
            return Err(failed(format!(
                "provider `{}` returned `{}@{}`, which it doesn't declare",
                self.name, draft.event_type, draft.v
            )));
        }
        if draft.event_type == WorkItemRecorded::TYPE {
            let item = draft.payload["item"]["ref"].as_str().unwrap_or_default();
            let owner = oxplow_domain::work_items::provider_of(item).ok();
            if owner != Some(self.spec.id.as_str()) {
                return Err(failed(format!(
                    "provider `{}` recorded `{item}`, which isn't one of its items",
                    self.name
                )));
            }
        }
        Envelope::new(draft.event_type, draft.v, actor.source(), draft.payload)
            .map(|e| e.with_subject(draft.subject))
            .map_err(|e| failed(e.to_string()))
    }
}

/// The instances and their health.
pub struct ProviderRegistry {
    deps: HostDeps,
    me: Weak<ProviderRegistry>,
    bus: Weak<CommandBus>,
    work_items: WorkItemsRegistry,
    running: tokio::sync::Mutex<BTreeMap<String, Arc<Instance>>>,
    health: parking_lot::Mutex<BTreeMap<String, InstanceHealth>>,
    /// One reconcile at a time.
    reconciling: tokio::sync::Mutex<()>,
}

impl ProviderRegistry {
    pub fn new(deps: HostDeps, bus: &Arc<CommandBus>, work_items: WorkItemsRegistry) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            deps,
            me: me.clone(),
            bus: Arc::downgrade(bus),
            work_items,
            running: tokio::sync::Mutex::new(BTreeMap::new()),
            health: parking_lot::Mutex::new(BTreeMap::new()),
            reconciling: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn get(&self, instance: &str) -> Option<Arc<Instance>> {
        self.running.lock().await.get(instance).cloned()
    }

    pub fn health(&self, instance: &str) -> Option<InstanceHealth> {
        self.health.lock().get(instance).cloned()
    }

    fn set_state(&self, instance: &str, state: InstanceState) {
        self.health
            .lock()
            .entry(instance.to_string())
            .or_insert_with(|| InstanceHealth::new(InstanceState::Off))
            .state = state;
    }

    /// The primary worktree's extensions.
    fn extensions(&self) -> Arc<Vec<Extension>> {
        self.deps.catalog.get(&self.deps.project_dir)
    }

    /// The enabled extension and spec behind `instance`.
    fn find(&self, instance: &str) -> Option<(Extension, ProviderSpec)> {
        self.extensions()
            .iter()
            .filter(|e| e.enabled)
            .find_map(|e| {
                e.providers
                    .iter()
                    .find(|s| s.approval_name(&e.name) == instance)
                    .map(|s| (e.clone(), s.clone()))
            })
    }

    fn instances_config(&self) -> BTreeMap<String, oxplow_config::ExtensionInstanceConfig> {
        crate::config_service::read_config(&self.deps.config).extension_instances
    }

    /// Every declared provider and every configured instance, with health.
    pub fn list(&self) -> Vec<ProviderInstanceView> {
        let configured = self.instances_config();
        let mut out: BTreeMap<String, ProviderInstanceView> = BTreeMap::new();
        for ext in self.extensions().iter().filter(|e| e.enabled) {
            for spec in &ext.providers {
                let instance = spec.approval_name(&ext.name);
                let cfg = configured.get(&instance);
                let dir = host::ext_dir(&self.deps.project_dir, ext);
                let config_schema = spec::read_declarations(spec, &|rel| {
                    std::fs::read_to_string(dir.join(rel)).ok()
                })
                .map(|d| d.config_schema)
                .unwrap_or(Value::Null);
                out.insert(
                    instance.clone(),
                    ProviderInstanceView {
                        extension: ext.name.clone(),
                        provider: spec.id.clone(),
                        capability: spec.capability.clone(),
                        enabled: cfg.is_some_and(|c| c.enabled),
                        config: cfg.map(|c| c.config.clone()).unwrap_or_else(|| json!({})),
                        config_schema,
                        approved: crate::exec_consent::may_run_provider(
                            &self.deps.approvals,
                            &self.deps.project_dir,
                            ext,
                            spec,
                        ),
                        credentials: spec
                            .credentials
                            .iter()
                            .map(|name| crate::source_runner::CredentialStatus {
                                name: name.clone(),
                                set: self
                                    .deps
                                    .secrets
                                    .get(&crate::source_runner::credential_account(
                                        &self.deps.project,
                                        &ext.name,
                                        name,
                                    ))
                                    .ok()
                                    .flatten()
                                    .is_some(),
                            })
                            .collect(),
                        health: self
                            .health(&instance)
                            .unwrap_or_else(|| InstanceHealth::new(InstanceState::Off)),
                        instance,
                    },
                );
            }
        }
        for (instance, cfg) in configured {
            out.entry(instance.clone())
                .or_insert_with(|| ProviderInstanceView {
                    extension: instance.split('/').next().unwrap_or_default().into(),
                    provider: instance.split('/').nth(1).unwrap_or_default().into(),
                    capability: String::new(),
                    enabled: cfg.enabled,
                    config: cfg.config.clone(),
                    config_schema: Value::Null,
                    approved: false,
                    credentials: Vec::new(),
                    health: InstanceHealth::new(InstanceState::Missing),
                    instance,
                });
        }
        out.into_values().collect()
    }

    /// Make the running instances match the project's config.
    pub async fn reconcile(&self) {
        let _one = self.reconciling.lock().await;
        let configured = self.instances_config();
        let running: Vec<String> = self.running.lock().await.keys().cloned().collect();
        for name in running {
            if !configured.get(&name).is_some_and(|c| c.enabled) {
                self.stop(&name).await;
            }
        }
        for (name, cfg) in configured {
            let Some((ext, spec)) = self.find(&name) else {
                self.stop(&name).await;
                self.set_state(&name, InstanceState::Missing);
                continue;
            };
            if !cfg.enabled {
                self.set_state(&name, InstanceState::Off);
                continue;
            }
            if let Some(reason) = self.disabled_reason(&name).await {
                self.stop(&name).await;
                self.set_state(&name, InstanceState::Disabled { reason });
                continue;
            }
            if let Some(current) = self.get(&name).await {
                if current.config == cfg.config
                    && current.spec == spec
                    && current.ext.path == ext.path
                {
                    continue;
                }
                self.stop(&name).await;
            }
            let _ = self.enable(&ext, &spec, cfg.config).await;
        }
    }

    /// Why `instance` is automatically disabled on this machine: its last
    /// `provider.disabled` has no later `provider.enabled`.
    async fn disabled_reason(&self, instance: &str) -> Option<String> {
        let instance = instance.to_string();
        self.deps
            .db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT type, json_extract(payload, '$.reason') FROM event_log
                     WHERE type IN ('provider.disabled', 'provider.enabled')
                       AND json_extract(payload, '$.instance') = ?1
                     ORDER BY seq DESC LIMIT 1",
                    [instance],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
            .ok()
            .flatten()
            .filter(|(t, _)| t == ProviderDisabled::TYPE)
            .map(|(_, reason)| reason.unwrap_or_default())
    }

    /// Check `ext`'s provider `spec` against `config` without enabling it:
    /// consent, spawn, handshake, `check`, then stop.
    pub async fn check(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        config: Value,
    ) -> Result<(), HostError> {
        let instance = self.instance(ext, spec, config)?;
        instance.start().await.map(|_| ())
    }

    fn instance(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        config: Value,
    ) -> Result<Arc<Instance>, HostError> {
        let name = spec.approval_name(&ext.name);
        let dir = host::ext_dir(&self.deps.project_dir, ext);
        let declared =
            spec::read_declarations(spec, &|rel| std::fs::read_to_string(dir.join(rel)).ok())
                .map_err(|message| HostError::Failed {
                    name: name.clone(),
                    message,
                })?;
        Ok(Arc::new(Instance {
            name,
            ext: ext.clone(),
            spec: spec.clone(),
            config,
            declared,
            deps: self.deps.clone(),
            registry: self.me.clone(),
            live: tokio::sync::Mutex::new(None),
            not_before: parking_lot::Mutex::new(None),
        }))
    }

    /// Start `ext`'s provider `spec` with `config` and register what it
    /// declares. Consent, a matching handshake and a clean `check` come
    /// first: refused, nothing is registered (a failed start counts
    /// towards [`FAILURES_TO_DISABLE`]).
    pub async fn enable(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        config: Value,
    ) -> Result<(), HostError> {
        let name = spec.approval_name(&ext.name);
        let refuse = |message: String| HostError::Failed {
            name: name.clone(),
            message,
        };
        let bus = self
            .bus
            .upgrade()
            .ok_or_else(|| refuse("the command bus is gone".into()))?;
        if self.get(&name).await.is_some() {
            return Err(refuse(format!("`{name}` is already running")));
        }
        if bus.has_namespace(&spec.id) || self.work_items.get(&spec.id).is_ok() {
            return Err(refuse(format!(
                "`{}` is already a command namespace or a provider",
                spec.id
            )));
        }
        let instance = self.instance(ext, spec, config)?;
        let schemas = bus.event_schemas();
        for t in &instance.declared.event_types {
            if schemas.schema(&t.event_type, t.v) != Some(&t.schema) {
                return Err(refuse(format!(
                    "it declares `{}@{}`, which isn't an event type oxplow knows with that schema \
                     (a provider emits core types only, for now)",
                    t.event_type, t.v
                )));
            }
        }
        self.set_state(&name, InstanceState::Checking);
        match instance.start().await {
            Ok(live) => {
                *instance.live.lock().await = Some(live);
                self.register(&bus, &instance).map_err(&refuse)?;
                self.running.lock().await.insert(name.clone(), instance);
                let mut health = self.health.lock();
                let h = health
                    .entry(name)
                    .or_insert_with(|| InstanceHealth::new(InstanceState::Ready));
                h.state = InstanceState::Ready;
                h.consecutive_failures = 0;
                h.last_ok_at = Some(now());
                Ok(())
            }
            // It may come up: enabled and failing, its next call restarts
            // it with backoff.
            Err(e @ HostError::Failed { .. }) => {
                self.register(&bus, &instance).map_err(&refuse)?;
                self.running.lock().await.insert(name.clone(), instance);
                self.failed(&name, e.to_string()).await;
                Ok(())
            }
            Err(e) => {
                self.start_failed(&name, e.clone()).await;
                Err(e)
            }
        }
    }

    /// Put `instance`'s commands on the bus and its capability provider
    /// in its registry; all or nothing.
    fn register(&self, bus: &Arc<CommandBus>, instance: &Arc<Instance>) -> Result<(), String> {
        let id = &instance.spec.id;
        for command in commands(instance)? {
            if let Err(e) = bus.register(command) {
                bus.unregister_namespace(id);
                return Err(e.to_string());
            }
        }
        if instance.spec.capability == spec::WORK_ITEMS {
            match super::work_items::ExternalWorkItems::new(bus, instance) {
                Ok(provider) => self.work_items.register(Arc::new(provider)),
                Err(e) => {
                    bus.unregister_namespace(id);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Stop `instance`: its commands and capability provider go, and its
    /// process dies. `false` when it wasn't running.
    pub async fn stop(&self, instance: &str) -> bool {
        let Some(running) = self.running.lock().await.remove(instance) else {
            return false;
        };
        if let Some(bus) = self.bus.upgrade() {
            bus.unregister_namespace(&running.spec.id);
        }
        self.work_items.unregister(&running.spec.id);
        running.live.lock().await.take();
        true
    }

    /// A person's Check / Enable / Disable on Settings → Integrations:
    /// write `extensionInstances.<instance>` through `config.set` and, to
    /// enable, run `provider.enable`. Enabling checks first: an
    /// unapproved or unconfigured instance is refused and nothing is
    /// written.
    pub async fn set_instance(
        &self,
        actor: &Actor,
        instance: &str,
        enabled: bool,
        config: Value,
    ) -> Result<ProviderInstanceView, CommandError> {
        let (ext, spec) = self.find(instance).ok_or_else(|| CommandError::Invalid {
            field: Some("/instance".into()),
            message: format!("no enabled extension declares provider `{instance}`"),
        })?;
        if enabled {
            match self.check(&ext, &spec, config.clone()).await {
                Ok(()) => {}
                Err(HostError::Unconfigured { problems, .. }) => {
                    let first = problems.first();
                    return Err(CommandError::Invalid {
                        field: Some(format!(
                            "/config{}",
                            first.map(|p| p.path.as_str()).unwrap_or_default()
                        )),
                        message: problems
                            .iter()
                            .map(|p| format!("{} {}", p.path, p.message))
                            .collect::<Vec<_>>()
                            .join("; "),
                    });
                }
                Err(e) => {
                    return Err(CommandError::Failed {
                        message: e.to_string(),
                    })
                }
            }
        }
        let bus = self.bus.upgrade().ok_or_else(|| CommandError::Failed {
            message: "the command bus is gone".into(),
        })?;
        let mut all = self.instances_config();
        all.insert(
            instance.to_string(),
            oxplow_config::ExtensionInstanceConfig { enabled, config },
        );
        bus.run(
            actor,
            crate::commands::config_commands::SET,
            json!({ "key": "extensionInstances", "value": all }),
            true,
        )
        .await?;
        if enabled {
            bus.run(actor, ENABLE, json!({ "instance": instance }), false)
                .await?;
        } else {
            self.reconcile().await;
        }
        self.view(instance)
    }

    /// Check `instance` with `config` for a person (Settings' Check): its
    /// view with the outcome as its state; nothing is enabled or written.
    pub async fn check_instance(
        &self,
        instance: &str,
        config: Value,
    ) -> Result<ProviderInstanceView, CommandError> {
        let (ext, spec) = self.find(instance).ok_or_else(|| CommandError::Invalid {
            field: Some("/instance".into()),
            message: format!("no enabled extension declares provider `{instance}`"),
        })?;
        let mut view = self.view(instance)?;
        view.health.state = match self.check(&ext, &spec, config).await {
            Ok(()) => InstanceState::Ready,
            Err(HostError::Unapproved(_)) => InstanceState::Unapproved,
            Err(HostError::Unconfigured { problems, .. }) => InstanceState::Unconfigured {
                problems: problems
                    .into_iter()
                    .map(|p| ConfigProblem {
                        path: p.path,
                        message: p.message,
                    })
                    .collect(),
            },
            Err(e) => InstanceState::Failing {
                errors: vec![e.to_string()],
            },
        };
        Ok(view)
    }

    /// A person enables `instance` again (`provider.enable`): its failure
    /// count and backoff start over.
    async fn reset(&self, instance: &str) {
        if let Some(h) = self.health.lock().get_mut(instance) {
            h.consecutive_failures = 0;
        }
        if let Some(i) = self.get(instance).await {
            *i.not_before.lock() = None;
        }
    }

    /// One instance's view.
    pub fn view(&self, instance: &str) -> Result<ProviderInstanceView, CommandError> {
        self.list()
            .into_iter()
            .find(|v| v.instance == instance)
            .ok_or_else(|| CommandError::Invalid {
                field: Some("/instance".into()),
                message: format!("no provider instance `{instance}`"),
            })
    }

    /// A start failed: set the state it says, counting a failure.
    async fn start_failed(&self, instance: &str, e: HostError) {
        match e {
            HostError::Unapproved(_) => {
                self.stop(instance).await;
                self.set_state(instance, InstanceState::Unapproved);
            }
            HostError::Unconfigured { problems, .. } => {
                self.stop(instance).await;
                self.set_state(
                    instance,
                    InstanceState::Unconfigured {
                        problems: problems
                            .into_iter()
                            .map(|p| ConfigProblem {
                                path: p.path,
                                message: p.message,
                            })
                            .collect(),
                    },
                );
            }
            // It isn't what was approved: off until a person looks.
            HostError::DeclarationsChanged { .. } => {
                self.disable(instance, e.to_string()).await;
            }
            HostError::Failed { .. } => self.failed(instance, e.to_string()).await,
        }
    }

    /// A start or call failed: count it; [`FAILURES_TO_DISABLE`] in a row
    /// disable the instance. The next start backs off meanwhile.
    async fn failed(&self, instance: &str, error: String) {
        let failures = {
            let mut health = self.health.lock();
            let h = health
                .entry(instance.to_string())
                .or_insert_with(|| InstanceHealth::new(InstanceState::Checking));
            h.consecutive_failures += 1;
            let mut errors = match &h.state {
                InstanceState::Failing { errors } => errors.clone(),
                _ => Vec::new(),
            };
            errors.push(error.clone());
            let excess = errors.len().saturating_sub(ERRORS_KEPT);
            errors.drain(..excess);
            h.state = InstanceState::Failing { errors };
            h.consecutive_failures
        };
        if failures >= FAILURES_TO_DISABLE {
            self.disable(
                instance,
                format!("{failures} failures in a row; the last: {error}"),
            )
            .await;
            return;
        }
        if let Some(i) = self.get(instance).await {
            let wait = self
                .deps
                .backoff
                .saturating_mul(1 << (failures - 1).min(6))
                .min(MAX_BACKOFF);
            *i.not_before.lock() = Some(Instant::now() + wait);
        }
    }

    /// Stop `instance` and keep it off on this machine until a person
    /// enables it: `provider.disabled@1`.
    async fn disable(&self, instance: &str, reason: String) {
        self.stop(instance).await;
        self.set_state(
            instance,
            InstanceState::Disabled {
                reason: reason.clone(),
            },
        );
        let event = Envelope::typed::<ProviderDisabled>(
            "system:providers",
            &ProviderDisabledV1 {
                instance: instance.to_string(),
                reason,
            },
        )
        .with_subject([plugin_ref(instance)]);
        if let Err(e) = self.deps.log.append(event).await {
            tracing::error!(instance, error = %e, "logging provider.disabled failed");
        }
    }

    fn call_succeeded(&self, instance: &str, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        let mut health = self.health.lock();
        let h = health
            .entry(instance.to_string())
            .or_insert_with(|| InstanceHealth::new(InstanceState::Ready));
        h.state = InstanceState::Ready;
        h.consecutive_failures = 0;
        h.last_ok_at = Some(now());
        h.mean_invoke_ms = Some(match h.mean_invoke_ms {
            None => ms,
            Some(mean) => mean + (ms - mean) / 10.0,
        });
    }
}

/// The ref of an instance's extension (`plugin:<extension>`).
fn plugin_ref(instance: &str) -> String {
    format!("plugin:{}", instance.split('/').next().unwrap_or_default())
}

fn now() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnableInput {
    /// `<extension>/<provider id>`.
    instance: String,
}

/// `provider.enable { instance }`: a person turns an instance back on
/// on this machine — clearing an automatic disable — and it starts when
/// the project's config enables it. Human only; logs `provider.enabled@1`.
pub fn enable_command(registry: &Arc<ProviderRegistry>) -> Command {
    let registry = Arc::downgrade(registry);
    Command::new(
        CommandSpec {
            name: ENABLE.into(),
            summary: "Enable an extension provider's instance on this machine again, clearing an \
                      automatic disable (runs the provider process, a system the bus doesn't own)."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(EnableInput))
                .expect("schema serializes"),
            invokers: Invokers::HUMAN_ONLY,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Experimental,
            atomicity: Atomicity::External,
            effect: CommandEffect::Write,
        },
        Handler::External(Arc::new(move |actor, input| {
            let registry = registry.clone();
            Box::pin(async move {
                let EnableInput { instance } =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let registry = registry.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "the provider registry is gone".into(),
                })?;
                if registry.find(&instance).is_none() {
                    return Err(CommandError::Invalid {
                        field: Some("/instance".into()),
                        message: format!("no enabled extension declares provider `{instance}`"),
                    });
                }
                // Logged first, so the reconcile below sees it cleared.
                let event = Envelope::typed::<ProviderEnabled>(
                    actor.source(),
                    &ProviderEnabledV1 {
                        instance: instance.clone(),
                    },
                )
                .with_subject([plugin_ref(&instance)]);
                registry.deps.log.append(event).await?;
                registry.reset(&instance).await;
                registry.reconcile().await;
                let view = registry.view(&instance)?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(view).expect("view serializes"),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("provider.enable is a valid command")
}

/// The instance's declared commands, as bus commands.
fn commands(instance: &Arc<Instance>) -> Result<Vec<Command>, String> {
    let id = instance.spec.id.clone();
    instance
        .declared
        .commands
        .iter()
        .map(|decl| {
            let spec = CommandSpec {
                name: format!("{id}.{}", decl.name),
                summary: format!("{} (provider `{}`)", decl.summary, instance.name),
                input_schema: decl.input_schema.clone(),
                invokers: Invokers::ALL,
                confirm: host::confirm_of(&decl.confirm)?,
                undoable: decl.undoable,
                lifecycle: Lifecycle::Experimental,
                atomicity: Atomicity::External,
                effect: host::effect_of(&decl.effect)?,
            };
            let (instance, verb, id) = (instance.clone(), decl.name.clone(), id.clone());
            Command::new(
                spec,
                Handler::External(Arc::new(move |actor, input| {
                    let (instance, verb, id) = (instance.clone(), verb.clone(), id.clone());
                    Box::pin(async move {
                        let out = instance.invoke(&verb, input).await?;
                        let events = out
                            .events
                            .into_iter()
                            .map(|d| instance.envelope(&actor, d))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(HandlerOutput {
                            result: out.result,
                            inverse: out.inverse.map(|c| CommandCall {
                                name: format!("{id}.{}", c.command),
                                input: c.input,
                            }),
                            events,
                            after_commit: None,
                        })
                    })
                })),
            )
            .map_err(|e| e.to_string())
        })
        .collect()
}

impl HostError {
    /// The config problems, when that's why it isn't running.
    pub fn problems(&self) -> &[oxplow_provider_protocol::model::Problem] {
        match self {
            HostError::Unconfigured { problems, .. } => problems,
            _ => &[],
        }
    }
}

/// Keep the instances matching the config: once at boot, then on every
/// config change (which is also how enabling or disabling an extension
/// arrives).
pub fn spawn_reconciler(state: Arc<crate::Services>) {
    let mut rx = state.events.subscribe();
    tokio::spawn(async move {
        state.providers.reconcile().await;
        loop {
            match rx.recv().await {
                Ok(crate::events::OxplowEvent::ConfigChanged) => {
                    state.providers.reconcile().await;
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    state.providers.reconcile().await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
