//! The running provider instances (`Services.providers`): one long-lived
//! process per enabled instance, restarted with backoff when it dies.
//!
//! Enabling an instance starts it (consent, spawn, handshake, `check` of
//! its config) and, once it answers, registers its declared commands on
//! the bus as `<id>.<name>` (`Atomicity::External`) and its capability's
//! provider (`ExternalWorkItems`) in the capability's registry. Every
//! restart goes through consent again: a provider whose files changed
//! since approval doesn't come back until a person approves it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use oxplow_ai::secrets::SecretStore;
use oxplow_domain::work_items::WorkItemsRegistry;
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandSpec, Envelope, Invokers, Lifecycle,
};
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, EventDraft, Handle, InitializeResult, InvokeParams,
    InvokeResult, Problem,
};
use oxplow_provider_protocol::ProtocolError;
use serde_json::Value;

use super::host::{self, Connection, HostError, Launch};
use super::spec::{self, ProviderSpec};
use crate::commands::{Command, CommandBus, Handler, HandlerOutput};
use crate::exec_consent::ApprovalStore;
use crate::extensions::Extension;

/// The longest a failed instance waits before its next start.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// What the registry needs from the host.
#[derive(Clone)]
pub struct HostDeps {
    /// The primary worktree: where providers run from.
    pub project_dir: PathBuf,
    /// The project's key (credentials are scoped by it).
    pub project: String,
    pub approvals: Arc<ApprovalStore>,
    pub secrets: Arc<dyn SecretStore>,
    /// Reads the host's environment (tests pass their own).
    pub host_env: host::HostEnv,
}

/// A started process and the handle its `check` returned.
struct Live {
    conn: Connection,
    handle: Handle,
}

#[derive(Default)]
struct Backoff {
    failures: u32,
    not_before: Option<Instant>,
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
    live: tokio::sync::Mutex<Option<Live>>,
    backoff: parking_lot::Mutex<Backoff>,
}

impl Instance {
    fn ext_dir(&self) -> PathBuf {
        host::ext_dir(&self.deps.project_dir, &self.ext)
    }

    /// Consent, spawn, handshake, check.
    async fn start(&self) -> Result<Live, HostError> {
        if let Some(wait) = self
            .backoff
            .lock()
            .not_before
            .and_then(|t| t.checked_duration_since(Instant::now()))
        {
            return Err(HostError::Failed {
                name: self.name.clone(),
                message: format!("failed to start; trying again in {}s", wait.as_secs() + 1),
            });
        }
        if !crate::exec_consent::may_run_provider(
            &self.deps.approvals,
            &self.deps.project_dir,
            &self.ext,
            &self.spec,
        ) {
            return Err(HostError::Unapproved(self.name.clone()));
        }
        let started = self.start_approved().await;
        let mut backoff = self.backoff.lock();
        match &started {
            Ok(_) => *backoff = Backoff::default(),
            Err(_) => {
                backoff.failures += 1;
                let wait = Duration::from_secs(1u64 << backoff.failures.min(6)).min(MAX_BACKOFF);
                backoff.not_before = Some(Instant::now() + wait);
            }
        }
        started
    }

    async fn start_approved(&self) -> Result<Live, HostError> {
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
                detail: "its declarations file changed since it was enabled; enable it again"
                    .into(),
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

    /// The running process's peer and handle, starting it when it isn't.
    async fn connection(&self) -> Result<(oxplow_provider_protocol::Peer, Handle), HostError> {
        let mut live = self.live.lock().await;
        if live.as_ref().is_none_or(|l| l.conn.peer.is_closed()) {
            *live = None;
            *live = Some(self.start().await?);
        }
        let l = live.as_ref().expect("started above");
        Ok((l.conn.peer.clone(), l.handle.clone()))
    }

    /// Run one of its declared commands.
    pub async fn invoke(&self, command: &str, input: Value) -> Result<InvokeResult, CommandError> {
        let (peer, handle) = self.connection().await.map_err(|e| CommandError::Failed {
            message: e.to_string(),
        })?;
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
        result.map_err(|e| self.command_error(e))
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
        if draft.event_type == oxplow_domain::events::schema::WorkItemRecorded::TYPE {
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

use oxplow_domain::events::schema::EventType as _;

/// The enabled instances, by provider id.
pub struct ProviderRegistry {
    deps: HostDeps,
    bus: Weak<CommandBus>,
    work_items: WorkItemsRegistry,
    instances: tokio::sync::Mutex<BTreeMap<String, Arc<Instance>>>,
}

impl ProviderRegistry {
    pub fn new(deps: HostDeps, bus: &Arc<CommandBus>, work_items: WorkItemsRegistry) -> Self {
        Self {
            deps,
            bus: Arc::downgrade(bus),
            work_items,
            instances: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Instance>> {
        self.instances.lock().await.get(id).cloned()
    }

    /// Start `ext`'s provider `spec` with `config`, and register what it
    /// declares. Refused — nothing registered — when it isn't approved,
    /// its declarations don't fit, it won't start or its config fails.
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
        let mut instances = self.instances.lock().await;
        if instances.contains_key(&spec.id) {
            return Err(refuse(format!("provider `{}` is already enabled", spec.id)));
        }
        if bus.has_namespace(&spec.id) || self.work_items.get(&spec.id).is_ok() {
            return Err(refuse(format!(
                "`{}` is already a command namespace or a provider",
                spec.id
            )));
        }
        let dir = host::ext_dir(&self.deps.project_dir, ext);
        let declared =
            spec::read_declarations(spec, &|rel| std::fs::read_to_string(dir.join(rel)).ok())
                .map_err(&refuse)?;
        let schemas = bus.event_schemas();
        for t in &declared.event_types {
            if schemas.schema(&t.event_type, t.v) != Some(&t.schema) {
                return Err(refuse(format!(
                    "it declares `{}@{}`, which isn't an event type oxplow knows with that schema \
                     (a provider emits core types only, for now)",
                    t.event_type, t.v
                )));
            }
        }
        let instance = Arc::new(Instance {
            name: name.clone(),
            ext: ext.clone(),
            spec: spec.clone(),
            config,
            declared,
            deps: self.deps.clone(),
            live: tokio::sync::Mutex::new(None),
            backoff: parking_lot::Mutex::new(Backoff::default()),
        });
        let first = instance.start().await?;
        *instance.live.lock().await = Some(first);
        for command in commands(&instance).map_err(&refuse)? {
            if let Err(e) = bus.register(command) {
                bus.unregister_namespace(&spec.id);
                return Err(refuse(e.to_string()));
            }
        }
        if spec.capability == spec::WORK_ITEMS {
            match super::work_items::ExternalWorkItems::new(&bus, &instance) {
                Ok(provider) => self.work_items.register(Arc::new(provider)),
                Err(e) => {
                    bus.unregister_namespace(&spec.id);
                    return Err(refuse(e));
                }
            }
        }
        instances.insert(spec.id.clone(), instance);
        Ok(())
    }

    /// Stop provider `id`: its commands and capability provider go, and
    /// its process dies.
    pub async fn disable(&self, id: &str) -> bool {
        let Some(instance) = self.instances.lock().await.remove(id) else {
            return false;
        };
        if let Some(bus) = self.bus.upgrade() {
            bus.unregister_namespace(id);
        }
        self.work_items.unregister(id);
        instance.live.lock().await.take();
        true
    }
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
    pub fn problems(&self) -> &[Problem] {
        match self {
            HostError::Unconfigured { problems, .. } => problems,
            _ => &[],
        }
    }
}
