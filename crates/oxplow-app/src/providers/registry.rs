//! The provider instances (`Services.providers`): one long-lived process
//! per enabled instance, its health, and restart with backoff.
//!
//! An instance is `<extension>/<provider id>`, configured in the
//! project's `extensionInstances` (`{ enabled, config }`, a person's key).
//! [`ProviderRegistry::reconcile`] makes the running set match it — at
//! boot and on every config change. Starting an instance checks consent,
//! spawns, handshakes and `check`s its config; only then is its
//! capability's provider (`ExternalWorkItems`) registered in the
//! capability's registry — its verbs run as the dispatching
//! `work_item.<verb>` — and its other declared commands on the bus as
//! `<id>.<name>` (`Atomicity::External`), under the namespace `<id>` it
//! then holds. Every restart goes through consent again.
//!
//! Health is per machine. [`InstanceHealth`] is the process's state; the
//! failure policy is the one every plugin contribution shares
//! ([`crate::plugin_health`]): a start or call that fails (not a refused
//! input) counts, three in a row stop the instance and log
//! `plugin.disabled@1`, and it stays off — across restarts, since
//! `plugin_health` says so — until a person runs `plugin.enable`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Weak};
use std::time::{Duration, Instant};

use oxplow_ai::secrets::SecretStore;
use oxplow_db::PluginKey;
use oxplow_db::{Database, SqliteEventLogStore};
use oxplow_domain::events::schema::{EventType as _, WorkItemRecorded};
use oxplow_domain::work_items::WorkItemsRegistry;
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandSpec, DomainError, Envelope, Invokers,
    Lifecycle,
};
use oxplow_provider_protocol::model::{
    method, CheckParams, CheckResult, EventDraft, Handle, InitializeResult, InvokeParams,
    InvokeResult,
};
use oxplow_provider_protocol::ProtocolError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::host::{self, Connection, HostError, Launch};
use super::oauth;
use super::spec::{self, ProviderSpec};
use crate::commands::{Command, CommandBus, Handler, HandlerOutput};
use crate::exec_consent::ApprovalStore;
use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::Extension;

/// The longest a failed instance waits before its next start.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The errors an instance's health keeps.
const ERRORS_KEPT: usize = 5;
/// The longest rate-limit wait a call or read sits through (then retries
/// once); a longer one fails the call — never counted as a failure.
pub const RATE_LIMIT_WAIT_MAX: Duration = Duration::from_secs(10);

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
    /// Where approved copies of providers are kept and run from, outside
    /// the repo (`host::approved_copy`).
    pub copies: PathBuf,
    /// How long a `check` or `invoke` may take before it is cancelled and
    /// counted as a failure.
    pub call_timeout: Duration,
    /// This machine's global config dir, where the person's global
    /// instances are kept (`instances.yaml`); none, there are none.
    pub global_dir: Option<PathBuf>,
    /// Where the renderer hears that a sign-in finished.
    pub events: crate::events::EventBus,
}

/// Whose an instance is (P9.B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The project's: in its `extensionInstances`, shared with the team;
    /// its credentials are this project's on this machine.
    Project,
    /// The person's: in this machine's `instances.yaml`, running in every
    /// project that has its extension; its credentials are set once.
    Global,
}

/// The keychain scope of a global instance's credentials (a project's is
/// the project's key).
const GLOBAL_CREDENTIALS: &str = "global";

/// The global instances as last read, and the file's time then.
#[derive(Default)]
struct GlobalFile {
    read_at: Option<std::time::SystemTime>,
    loaded: bool,
    instances: BTreeMap<String, oxplow_config::ExtensionInstanceConfig>,
    /// The file's time at the last reconcile.
    reconciled_at: Option<std::time::SystemTime>,
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
    /// Configured, but it names no provider an enabled extension
    /// declares; `reason` says what's wrong and how to fix it.
    Missing {
        reason: String,
    },
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
    /// Its failures in a row (`plugin_health`'s count, shown here).
    pub consecutive_failures: u32,
    /// RFC 3339.
    pub last_ok_at: Option<String>,
    /// A moving average of its successful calls.
    pub mean_invoke_ms: Option<f64>,
    /// The provider's service said to wait until then (RFC 3339): a rate
    /// limit, which never counts as a failure. Cleared by the next
    /// success.
    pub rate_limited_until: Option<String>,
    /// What a read in progress last said (`$/progress`); `None` between
    /// reads.
    pub activity: Option<String>,
}

impl InstanceHealth {
    fn new(state: InstanceState) -> Self {
        Self {
            state,
            consecutive_failures: 0,
            last_ok_at: None,
            mean_invoke_ms: None,
            rate_limited_until: None,
            activity: None,
        }
    }
}

/// One of an instance's credentials, as Settings → Integrations shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCredential {
    pub name: String,
    /// This machine has a value for it (a signed-in one: a token).
    pub set: bool,
    /// For one the person signs in for, where that stands; none, its
    /// value is pasted.
    pub sign_in: Option<oauth::SignInState>,
}

/// An instance as Settings → Integrations shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstanceView {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// The project's, or the person's on this machine (every project).
    pub scope: Scope,
    /// A global instance this project's own entry replaces here.
    pub overridden: bool,
    pub extension: String,
    /// The provider (its program) this is an instance of.
    pub provider: String,
    /// Its id: its refs' segment, its commands' namespace, what
    /// `activeProviders` names. A provider's default instance has the
    /// provider's id.
    pub instance_id: String,
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
    /// Each credential it declares and where it stands on this machine.
    pub credentials: Vec<InstanceCredential>,
    pub health: InstanceHealth,
    /// Each collector it declares and where its reads stand (P7.A3).
    pub collectors: Vec<CollectorView>,
}

/// A provider's collector as Settings → Integrations shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorView {
    pub name: String,
    pub entity: String,
    /// `never`, `reading`, `ok` or `error`.
    pub status: String,
    pub error: Option<String>,
    /// RFC 3339.
    pub last_read_at: Option<String>,
    /// Records its reads have delivered.
    pub records: i64,
}

/// A started process and the handle its `check` returned.
struct Live {
    conn: Connection,
    handle: Handle,
    /// When it started: a call older than it wasn't refused by it.
    since: Instant,
}

/// One enabled instance.
pub struct Instance {
    /// `<extension>/<instance id>`.
    pub name: String,
    /// The instance's id: its refs' segment (`work_item:<id>:…`), its
    /// commands' namespace, its capability provider's id. A provider's
    /// default instance has the provider's.
    pub id: String,
    /// Whose it is: where its credentials are kept.
    pub scope: Scope,
    pub ext: Extension,
    pub spec: ProviderSpec,
    pub config: Value,
    /// The approved declarations it was enabled with.
    pub declared: InitializeResult,
    pub(super) deps: HostDeps,
    pub(super) registry: Weak<ProviderRegistry>,
    live: tokio::sync::Mutex<Option<Live>>,
    /// One start at a time; held across a start, which `live` never is.
    starting: tokio::sync::Mutex<()>,
    not_before: parking_lot::Mutex<Option<Instant>>,
    /// One read per collector at a time (tsk715): a second waits, then
    /// resumes from the checkpoint the first left.
    pub(super) reading:
        parking_lot::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Instance {
    /// Whether any of its credentials is one the person signs in for.
    fn signs_in(&self) -> bool {
        self.spec.credentials.iter().any(|c| c.oauth.is_some())
    }

    /// The keychain account of its credential `name`.
    fn account(&self, name: &str) -> String {
        crate::collector_runner::instance_credential_account(
            credential_scope(&self.deps, self.scope),
            &self.ext.name,
            &self.id,
            name,
        )
    }

    /// What its process is given: each pasted credential this machine has
    /// a value for, and each signed-in one's access token — renewed first
    /// when it is about to lapse or, with `renew`, because its service
    /// refused it. A client secret is the host's to send with a token
    /// request, never the process's. A signed-in credential with no token
    /// to give is a problem with the instance, at `/credentials/<NAME>`.
    async fn credentials(&self, renew: bool) -> Result<BTreeMap<String, String>, HostError> {
        let failed = |name: &str, why: String| HostError::Failed {
            name: self.name.clone(),
            message: format!("credential `{name}`: {why}"),
        };
        let problem = |name: &str, message: String| oxplow_provider_protocol::model::Problem {
            path: format!("/credentials/{name}"),
            message,
        };
        let secrets = self.deps.secrets.as_ref();
        let mut out = BTreeMap::new();
        let mut problems = Vec::new();
        for c in &self.spec.credentials {
            let name = c.name.as_str();
            let Some(decl) = &c.oauth else {
                if self.spec.is_client_secret(name) {
                    continue;
                }
                if let Some(value) = secrets
                    .get(&self.account(name))
                    .map_err(|e| failed(name, e.to_string()))?
                {
                    out.insert(name.to_string(), value);
                }
                continue;
            };
            let client_secret = match &decl.client_secret {
                None => None,
                Some(secret) => match secrets
                    .get(&self.account(secret))
                    .map_err(|e| failed(secret, e.to_string()))?
                {
                    Some(value) => Some(value),
                    None => {
                        problems.push(problem(
                            secret,
                            format!("isn't set — `{name}`'s sign-in needs it"),
                        ));
                        continue;
                    }
                },
            };
            match oauth::access_token(
                secrets,
                &self.account(name),
                decl,
                client_secret.as_deref(),
                renew,
            )
            .await
            {
                Ok(token) => {
                    out.insert(name.to_string(), token);
                }
                Err(oauth::CredentialProblem::NotSignedIn) => problems.push(problem(
                    name,
                    "isn't signed in — sign in on Settings → Integrations".into(),
                )),
                Err(oauth::CredentialProblem::SignInAgain(why)) => problems.push(problem(
                    name,
                    format!(
                        "its sign-in is no longer good ({why}) — sign in again on Settings → \
                         Integrations"
                    ),
                )),
                Err(oauth::CredentialProblem::Failed(why)) => return Err(failed(name, why)),
            }
        }
        if problems.is_empty() {
            Ok(out)
        } else {
            Err(HostError::Unconfigured {
                name: self.name.clone(),
                problems,
            })
        }
    }

    /// Consent (a verified copy of the approved folder), spawn from that
    /// copy, handshake, check. A `check` its service refuses for its
    /// credentials (`Auth`) is tried once more on renewed tokens, when it
    /// has any to renew.
    async fn start(&self) -> Result<Live, HostError> {
        match self.start_once(false).await {
            Err((_, true)) if self.signs_in() => self.start_once(true).await.map_err(|(e, _)| e),
            other => other.map_err(|(e, _)| e),
        }
    }

    /// One start; the error says whether it was its `check` answering
    /// `Auth`.
    async fn start_once(&self, renew: bool) -> Result<Live, (HostError, bool)> {
        let plain = |e: HostError| (e, false);
        let copy = copy_approved(&self.deps, &self.ext, &self.spec)
            .await
            .map_err(plain)?;
        if copy.declared != self.declared {
            return Err(plain(HostError::DeclarationsChanged {
                name: self.name.clone(),
                detail: "its declarations changed since it was enabled".into(),
            }));
        }
        let (dir, declared) = (copy.ext_dir, copy.declared);
        let credentials = self.credentials(renew).await.map_err(plain)?;
        let names: Vec<String> = credentials.keys().cloned().collect();
        let conn = host::connect(&Launch {
            name: self.name.clone(),
            instance_id: self.id.clone(),
            ext_dir: dir,
            spec: self.spec.clone(),
            declared,
            credentials,
            host_env: self.deps.host_env.clone(),
        })
        .await
        .map_err(plain)?;
        let since = Instant::now();
        let checked: CheckResult = call_within(
            &conn.peer,
            method::CHECK,
            &CheckParams {
                config: self.config.clone(),
                credentials: names,
            },
            self.deps.call_timeout,
        )
        .await
        .map_err(|e| {
            let auth = matches!(e, ProtocolError::Auth(_));
            (
                HostError::Failed {
                    name: self.name.clone(),
                    message: format!("check: {e}"),
                },
                auth,
            )
        })?;
        match checked.handle {
            Some(handle) if checked.problems.is_empty() => Ok(Live {
                conn,
                handle,
                since,
            }),
            _ => Err(plain(HostError::Unconfigured {
                name: self.name.clone(),
                problems: checked.problems,
            })),
        }
    }

    /// Its service refused its credentials (`Auth`) on a call made at
    /// `called`: renew each signed-in credential's token and end the
    /// process, so the next call starts on them. False when it has none
    /// to renew. A process started since `called` is already on newer
    /// tokens than the refused call's and is left alone.
    pub(super) async fn reauthorize(&self, called: Instant) -> bool {
        if !self.signs_in() {
            return false;
        }
        let _one = self.starting.lock().await;
        {
            let mut live = self.live.lock().await;
            if live.as_ref().is_some_and(|l| l.since > called) {
                return true;
            }
            live.take();
        }
        // What can't be renewed shows when it next starts (its `check`
        // names the credential); here the only question is whether to
        // try again.
        if let Err(e) = self.credentials(true).await {
            tracing::info!(instance = %self.name, error = %e, "renewing its sign-in failed");
        }
        true
    }

    /// The running process's peer and handle, starting it when it isn't
    /// (unless it's backing off). The start runs without holding `live`,
    /// so a start that hangs never blocks `stop`.
    pub(super) async fn connection(
        &self,
    ) -> Result<(oxplow_provider_protocol::Peer, Handle), CommandError> {
        {
            let live = self.live.lock().await;
            if let Some(l) = live.as_ref().filter(|l| !l.conn.peer.is_closed()) {
                return Ok((l.conn.peer.clone(), l.handle.clone()));
            }
        }
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
        let _one = self.starting.lock().await;
        // Another caller may have started it while this one waited.
        if let Some(l) = self
            .live
            .lock()
            .await
            .as_ref()
            .filter(|l| !l.conn.peer.is_closed())
        {
            return Ok((l.conn.peer.clone(), l.handle.clone()));
        }
        match self.start().await {
            Ok(l) => {
                *self.not_before.lock() = None;
                let out = (l.conn.peer.clone(), l.handle.clone());
                *self.live.lock().await = Some(l);
                Ok(out)
            }
            Err(e) => {
                let message = e.to_string();
                if let Some(r) = self.registry.upgrade() {
                    r.start_failed(&self.name, e).await;
                }
                Err(CommandError::Failed { message })
            }
        }
    }

    /// Send the running process a `fake/hooks` notification (the fake
    /// provider's script hooks, mid-session).
    #[cfg(test)]
    pub(super) async fn hook(&self, hooks: &str) {
        let (peer, _) = self.connection().await.expect("it runs");
        peer.notify("fake/hooks", json!({ "hooks": hooks }))
            .await
            .expect("the hooks reach it");
    }

    /// Forget the process `peer` talks to when it died under a call, so
    /// the next call restarts it.
    pub(super) async fn forget_if_closed(&self, peer: &oxplow_provider_protocol::Peer) {
        if peer.is_closed() {
            self.live.lock().await.take();
        }
    }

    /// Run one of its declared commands.
    pub async fn invoke(&self, command: &str, input: Value) -> Result<InvokeResult, CommandError> {
        let mut retried = false;
        let mut reauthorized = false;
        loop {
            let (peer, handle) = self.connection().await?;
            let started = Instant::now();
            let result = call_within::<_, InvokeResult>(
                &peer,
                method::INVOKE,
                &InvokeParams {
                    handle,
                    command: command.into(),
                    input: input.clone(),
                },
                self.deps.call_timeout,
            )
            .await;
            // It died under the call: the next one restarts it.
            self.forget_if_closed(&peer).await;
            if let Err(ProtocolError::RateLimited {
                message,
                retry_after_ms,
            }) = &result
            {
                match self.rate_limited(message, *retry_after_ms, retried).await {
                    Some(err) => return Err(err),
                    None => {
                        retried = true;
                        continue;
                    }
                }
            }
            // Its service refused its credentials: once, on renewed ones.
            if matches!(result, Err(ProtocolError::Auth(_)))
                && !reauthorized
                && self.reauthorize(started).await
            {
                reauthorized = true;
                continue;
            }
            return self.after_call(result, started).await;
        }
    }

    /// A rate limit: noted on its health (never counted as a failure).
    /// `None` when it was short enough to have been waited out — retry,
    /// once; else the error that says when to try again.
    pub(super) async fn rate_limited(
        &self,
        message: &str,
        retry_after_ms: Option<u64>,
        retried: bool,
    ) -> Option<CommandError> {
        let wait = retry_after_ms.map(Duration::from_millis);
        if let Some(r) = self.registry.upgrade() {
            r.note_rate_limit(&self.name, wait);
        }
        match wait {
            Some(wait) if !retried && wait <= RATE_LIMIT_WAIT_MAX => {
                tokio::time::sleep(wait).await;
                None
            }
            _ => Some(CommandError::Failed {
                message: format!(
                    "provider `{}` is rate limited ({message}){}",
                    self.name,
                    match wait {
                        Some(w) => format!("; try again in {}s", w.as_secs().max(1)),
                        None => String::new(),
                    }
                ),
            }),
        }
    }

    async fn after_call(
        &self,
        result: Result<InvokeResult, ProtocolError>,
        started: Instant,
    ) -> Result<InvokeResult, CommandError> {
        let registry = self.registry.upgrade();
        match result {
            Ok(out) => {
                if let Some(r) = registry {
                    r.call_succeeded(self, started.elapsed()).await;
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
                    r.call_failed(self, err.to_string()).await;
                }
                Err(err)
            }
        }
    }

    pub(super) fn command_error(&self, e: ProtocolError) -> CommandError {
        match e {
            ProtocolError::InvalidInput { field, message } => CommandError::Invalid {
                field: Some(field),
                message: format!("{}: {message}", self.id),
            },
            other => CommandError::Failed {
                message: format!("provider `{}`: {other}", self.name),
            },
        }
    }

    /// A returned event as the envelope the bus logs: a type it declared,
    /// and (for a work-item record) an item of its own.
    pub(crate) fn envelope(
        &self,
        actor: &Actor,
        draft: EventDraft,
    ) -> Result<Envelope, CommandError> {
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
            if owner != Some(self.id.as_str()) {
                return Err(failed(format!(
                    "provider `{}` recorded `{item}`, which isn't one of its items",
                    self.name
                )));
            }
        }
        for subject in &draft.subject {
            check_subject(&self.id, &self.ext.name, subject).map_err(&failed)?;
        }
        Envelope::new(draft.event_type, draft.v, actor.source(), draft.payload)
            .map(|e| e.with_subject(draft.subject))
            .map_err(|e| failed(e.to_string()))
    }
}

/// Whether an instance's event may name `subject`: only its own items
/// (`work_item:<instance id>:…`) and its extension (`plugin:<ext>`).
pub fn check_subject(provider: &str, extension: &str, subject: &str) -> Result<(), String> {
    let own_item = subject.starts_with("work_item:")
        && oxplow_domain::work_items::provider_of(subject).ok() == Some(provider);
    if own_item || subject == format!("plugin:{extension}") {
        Ok(())
    } else {
        Err(format!(
            "provider `{extension}/{provider}` named `{subject}`, which isn't one of its own refs"
        ))
    }
}

/// The instances and their health.
pub struct ProviderRegistry {
    pub(super) deps: HostDeps,
    me: Weak<ProviderRegistry>,
    pub(super) bus: Weak<CommandBus>,
    work_items: WorkItemsRegistry,
    pub(super) running: tokio::sync::Mutex<BTreeMap<String, Arc<Instance>>>,
    health: parking_lot::Mutex<BTreeMap<String, InstanceHealth>>,
    /// One reconcile at a time.
    reconciling: tokio::sync::Mutex<()>,
    /// How many times each instance has been disabled: a start that began
    /// before a disable doesn't register what the disable stopped.
    disables: parking_lot::Mutex<BTreeMap<String, u64>>,
    /// The failure policy instances share with every plugin contribution.
    pub(super) plugins: crate::plugin_health::PluginHealth,
    /// This machine's global instances, re-read when their file changes.
    global: parking_lot::Mutex<GlobalFile>,
    /// Sign-ins under way, by `(instance, credential)`: a newer one for
    /// the same credential replaces the older.
    sign_ins: parking_lot::Mutex<BTreeMap<(String, String), tokio::task::JoinHandle<()>>>,
}

/// Where `scope`'s credentials are kept: the project's key, or the
/// machine's.
fn credential_scope(deps: &HostDeps, scope: Scope) -> &str {
    match scope {
        Scope::Project => &deps.project,
        Scope::Global => GLOBAL_CREDENTIALS,
    }
}

/// An instance's `plugin_health` key: `<extension>/<instance id>`.
pub(super) fn plugin_key(instance: &str) -> PluginKey {
    let (plugin, contribution) = instance.split_once('/').unwrap_or((instance, ""));
    PluginKey {
        plugin: plugin.to_string(),
        contribution: contribution.to_string(),
        kind: "provider",
    }
}

/// What an instance name resolves to.
pub(crate) struct Resolved {
    pub ext: Extension,
    pub spec: ProviderSpec,
    /// The instance's id.
    pub id: String,
    /// Whose it is.
    pub scope: Scope,
}

fn listed_or_none(ids: &[&str]) -> String {
    if ids.is_empty() {
        "it declares none".into()
    } else {
        format!("it declares {}", ids.join(", "))
    }
}

impl ProviderRegistry {
    pub fn new(deps: HostDeps, bus: &Arc<CommandBus>, work_items: WorkItemsRegistry) -> Arc<Self> {
        let plugins =
            crate::plugin_health::PluginHealth::new(deps.db.clone(), deps.log.vocabulary().clone());
        Arc::new_cyclic(|me| Self {
            plugins,
            deps,
            me: me.clone(),
            bus: Arc::downgrade(bus),
            work_items,
            running: tokio::sync::Mutex::new(BTreeMap::new()),
            health: parking_lot::Mutex::new(BTreeMap::new()),
            reconciling: tokio::sync::Mutex::new(()),
            disables: parking_lot::Mutex::new(BTreeMap::new()),
            global: parking_lot::Mutex::new(GlobalFile::default()),
            sign_ins: parking_lot::Mutex::new(BTreeMap::new()),
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

    /// The enabled extension, provider and instance id behind `instance`
    /// (`<extension>/<instance id>`): the provider its config entry names
    /// (`provider:`), else the one whose id the instance has — a
    /// provider's default instance. `Err` says what's missing and how to
    /// fix it.
    pub(crate) fn resolve(&self, instance: &str) -> Result<Resolved, String> {
        let provider = self
            .instances_config()
            .get(instance)
            .and_then(|c| c.provider.clone());
        self.resolve_as(instance, provider.as_deref())
    }

    /// [`Self::resolve`] for an instance of `provider` (none: the one
    /// whose id the instance has), whatever the config says.
    fn resolve_as(&self, instance: &str, provider: Option<&str>) -> Result<Resolved, String> {
        self.resolve_inner(instance, provider)
            .map_err(|why| format!("`{instance}`: {why}"))
    }

    fn resolve_inner(&self, instance: &str, provider: Option<&str>) -> Result<Resolved, String> {
        let (ext_name, id) = instance
            .split_once('/')
            .filter(|(e, i)| !e.is_empty() && !i.is_empty())
            .ok_or_else(|| "an instance is `<extension>/<instance id>`".to_string())?;
        let extensions = self.extensions();
        let ext = extensions
            .iter()
            .find(|e| e.enabled && e.name == ext_name)
            .ok_or_else(|| format!("no enabled extension `{ext_name}`"))?;
        let wanted = provider.unwrap_or(id);
        let declared: Vec<&str> = ext.providers.iter().map(|p| p.id.as_str()).collect();
        let spec = ext
            .providers
            .iter()
            .find(|p| p.id == wanted)
            .ok_or_else(|| {
                let which = match declared.as_slice() {
                    [] => format!("`{ext_name}` declares no provider"),
                    [one] => format!("add `provider: {one}`"),
                    many => format!("add `provider: <one of {}>`", many.join(", ")),
                };
                match provider {
                    Some(p) => format!(
                        "`{ext_name}` declares no provider `{p}` ({})",
                        listed_or_none(&declared)
                    ),
                    None => format!(
                    "`{ext_name}` declares no provider `{id}`; an instance with its own id says \
                     which provider it is — {which} to `extensionInstances.{instance}`"
                ),
                }
            })?;
        Ok(Resolved {
            ext: ext.clone(),
            spec: spec.clone(),
            id: id.to_string(),
            scope: self.scope_of(instance).0,
        })
    }

    /// The enabled extension and spec behind `instance`.
    pub(crate) fn find(&self, instance: &str) -> Option<(Extension, ProviderSpec)> {
        self.resolve(instance).ok().map(|r| (r.ext, r.spec))
    }

    /// The instances in effect here: this machine's global ones, each
    /// replaced whole by the project's entry of the same name.
    pub(super) fn instances_config(
        &self,
    ) -> BTreeMap<String, oxplow_config::ExtensionInstanceConfig> {
        let mut all = self.global_instances();
        all.extend(self.project_instances());
        all
    }

    /// The project's own entries (`extensionInstances`).
    fn project_instances(&self) -> BTreeMap<String, oxplow_config::ExtensionInstanceConfig> {
        crate::config_service::read_config(&self.deps.config).extension_instances
    }

    /// When the machine's instances file last changed (none: no file).
    fn global_mtime(&self) -> Option<std::time::SystemTime> {
        let dir = self.deps.global_dir.as_ref()?;
        std::fs::metadata(dir.join(oxplow_config::INSTANCES_FILE))
            .and_then(|m| m.modified())
            .ok()
    }

    /// This machine's global instances, re-read when their file changed.
    /// A file that doesn't load keeps what was last read (and says so).
    fn global_instances(&self) -> BTreeMap<String, oxplow_config::ExtensionInstanceConfig> {
        let Some(dir) = self.deps.global_dir.as_ref() else {
            return BTreeMap::new();
        };
        let mtime = self.global_mtime();
        let mut file = self.global.lock();
        if !file.loaded || file.read_at != mtime {
            match oxplow_config::GlobalInstances::load(dir) {
                Ok(read) => file.instances = read.instances,
                Err(error) => {
                    tracing::warn!(%error, "the global instances file didn't load; keeping what was last read")
                }
            }
            file.loaded = true;
            file.read_at = mtime;
        }
        file.instances.clone()
    }

    /// Whose `instance` is, and whether a project entry replaces a global
    /// one here. An instance configured nowhere is the project's.
    fn scope_of(&self, instance: &str) -> (Scope, bool) {
        if self.global_instances().contains_key(instance) {
            (
                Scope::Global,
                self.project_instances().contains_key(instance),
            )
        } else {
            (Scope::Project, false)
        }
    }

    /// The machine's instances file changed since the last reconcile
    /// (another project's oxplow wrote it): reconcile. Called on the sync
    /// timer; `true` when it did.
    pub async fn reconcile_if_global_changed(&self) -> bool {
        if self.global.lock().reconciled_at == self.global_mtime() {
            return false;
        }
        self.reconcile().await;
        true
    }

    /// What approving `instance` as it is on disk would change against
    /// what was approved last — the approved copy a start last ran
    /// (`host::last_approved`), whether or not it runs now (P6b.E3): its
    /// grants, each declared command and its features. Never run,
    /// everything is new. Reads files; runs nothing.
    pub async fn declaration_effects(
        &self,
        instance: &str,
    ) -> Result<crate::extension_effects::ProviderEffect, DomainError> {
        let Resolved { ext, spec, .. } =
            self.resolve(instance).map_err(|_| DomainError::NotFound)?;
        let dir = host::ext_dir(&self.deps.project_dir, &ext);
        let read = |rel: &str| std::fs::read_to_string(dir.join(rel)).ok();
        spec::read_declarations(&spec, &read).map_err(DomainError::Invalid)?;
        let on_disk = spec::DeclaredProvider::read(&spec, &read);
        let (copies, ext_c, spec_c) = (self.deps.copies.clone(), ext.clone(), spec.clone());
        let approved =
            tokio::task::spawn_blocking(move || host::last_approved(&copies, &ext_c, &spec_c))
                .await
                .map_err(|e| DomainError::Invariant(format!("reading the approved copy: {e}")))?;
        crate::extension_effects::providers_diff(
            &approved.into_iter().collect::<Vec<_>>(),
            &[on_disk],
        )
        .into_iter()
        .next()
        .ok_or(DomainError::NotFound)
    }

    /// One instance's view, before its collectors' read states.
    fn view_of(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        id: &str,
        cfg: Option<&oxplow_config::ExtensionInstanceConfig>,
    ) -> ProviderInstanceView {
        let instance = spec::instance_name(&ext.name, id);
        let (scope, overridden) = self.scope_of(&instance);
        let dir = host::ext_dir(&self.deps.project_dir, ext);
        let declared =
            spec::read_declarations(spec, &|rel| std::fs::read_to_string(dir.join(rel)).ok()).ok();
        let config_schema = declared
            .as_ref()
            .map(|d| d.config_schema.clone())
            .unwrap_or(Value::Null);
        let collectors = declared
            .map(|d| {
                d.collectors
                    .into_iter()
                    .map(|c| CollectorView {
                        name: c.name,
                        entity: c.entity,
                        status: "never".into(),
                        error: None,
                        last_read_at: None,
                        records: 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        ProviderInstanceView {
            scope,
            overridden,
            extension: ext.name.clone(),
            provider: spec.id.clone(),
            instance_id: id.to_string(),
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
                .map(|c| {
                    let account = crate::collector_runner::instance_credential_account(
                        credential_scope(&self.deps, scope),
                        &ext.name,
                        id,
                        &c.name,
                    );
                    let secrets = self.deps.secrets.as_ref();
                    let sign_in = c.oauth.as_ref().map(|_| oauth::state(secrets, &account));
                    InstanceCredential {
                        name: c.name.clone(),
                        set: match &sign_in {
                            Some(state) => {
                                matches!(state, oauth::SignInState::SignedIn { .. })
                            }
                            None => secrets.get(&account).ok().flatten().is_some(),
                        },
                        sign_in,
                    }
                })
                .collect(),
            health: self
                .health(&instance)
                .unwrap_or_else(|| InstanceHealth::new(InstanceState::Off)),
            collectors,
            instance,
        }
    }

    /// Every declared provider's default instance and every configured
    /// instance, with health.
    pub async fn list(&self) -> Vec<ProviderInstanceView> {
        let configured = self.instances_config();
        let mut out: BTreeMap<String, ProviderInstanceView> = BTreeMap::new();
        for ext in self.extensions().iter().filter(|e| e.enabled) {
            for spec in &ext.providers {
                let instance = spec::instance_name(&ext.name, &spec.id);
                // A config entry under a provider's own id that names
                // another provider is that other provider's instance.
                if configured
                    .get(&instance)
                    .is_some_and(|c| c.provider.as_deref().is_some_and(|p| p != spec.id))
                {
                    continue;
                }
                let view = self.view_of(ext, spec, &spec.id, configured.get(&instance));
                out.insert(instance, view);
            }
        }
        for (instance, cfg) in &configured {
            if out.contains_key(instance) {
                continue;
            }
            let view = match self.resolve(instance) {
                Ok(r) => self.view_of(&r.ext, &r.spec, &r.id, Some(cfg)),
                Err(reason) => {
                    let (extension, id) = instance.split_once('/').unwrap_or((instance, ""));
                    // An instance of an extension this project doesn't
                    // have is the person's elsewhere: not this project's
                    // to list.
                    let (scope, overridden) = self.scope_of(instance);
                    if scope == Scope::Global && !overridden {
                        continue;
                    }
                    ProviderInstanceView {
                        scope,
                        overridden,
                        extension: extension.into(),
                        provider: cfg.provider.clone().unwrap_or_else(|| id.into()),
                        instance_id: id.into(),
                        capability: String::new(),
                        enabled: cfg.enabled,
                        config: cfg.config.clone(),
                        config_schema: Value::Null,
                        approved: false,
                        credentials: Vec::new(),
                        health: InstanceHealth::new(InstanceState::Missing { reason }),
                        collectors: Vec::new(),
                        instance: instance.clone(),
                    }
                }
            };
            out.insert(instance.clone(), view);
        }
        let store = oxplow_db::SqliteProviderCollectorStore::new(self.deps.db.clone());
        for view in out.values_mut() {
            let states = store.for_instance(&view.instance).await.unwrap_or_default();
            for c in &mut view.collectors {
                if let Some(st) = states.iter().find(|s| s.collector == c.name) {
                    c.status = st.status.clone();
                    c.error = st.error.clone();
                    c.last_read_at = st.last_read_at.clone();
                    c.records = st.records;
                }
            }
        }
        out.into_values().collect()
    }

    /// Make the running instances match the project's config.
    pub async fn reconcile(&self) {
        let _one = self.reconciling.lock().await;
        let seen = self.global_mtime();
        let configured = self.instances_config();
        self.global.lock().reconciled_at = seen;
        let running: Vec<String> = self.running.lock().await.keys().cloned().collect();
        for name in running {
            if !configured.get(&name).is_some_and(|c| c.enabled) {
                self.stop(&name).await;
            }
        }
        for (name, cfg) in configured {
            let Resolved {
                ext,
                spec,
                id,
                scope,
            } = match self.resolve(&name) {
                Ok(resolved) => resolved,
                Err(reason) => {
                    self.stop(&name).await;
                    self.set_state(&name, InstanceState::Missing { reason });
                    continue;
                }
            };
            if !cfg.enabled {
                self.set_state(&name, InstanceState::Off);
                continue;
            }
            match self.disabled_reason(&name).await {
                Ok(None) => {}
                Ok(Some(reason)) => {
                    self.stop(&name).await;
                    self.set_state(&name, InstanceState::Disabled { reason });
                    continue;
                }
                // Not knowing is not a yes: it stays off.
                Err(e) => {
                    self.stop(&name).await;
                    self.set_state(
                        &name,
                        InstanceState::Failing {
                            errors: vec![format!("couldn't read whether it was disabled: {e}")],
                        },
                    );
                    continue;
                }
            }
            if let Some(current) = self.get(&name).await {
                if current.config == cfg.config
                    && current.spec == spec
                    && current.ext.path == ext.path
                    && current.scope == scope
                {
                    continue;
                }
                self.stop(&name).await;
            }
            let _ = self
                .enable_scoped(&ext, &spec, &id, scope, cfg.config)
                .await;
        }
        // The config may name another active provider (P7.A2).
        let config = crate::config_service::read_config(&self.deps.config);
        if let Err(e) =
            crate::capabilities::apply_active(&config, &self.work_items, &self.deps.db).await
        {
            tracing::warn!(error = %e, "restating the active providers failed");
        }
    }

    /// Why `instance` is automatically disabled on this machine
    /// (`plugin_health`).
    async fn disabled_reason(&self, instance: &str) -> Result<Option<String>, DomainError> {
        self.plugins.disabled_reason(&plugin_key(instance)).await
    }

    /// Check `ext`'s provider `spec` against `config` without enabling it
    /// — as its default instance: consent, spawn, handshake, `check`, then
    /// stop.
    pub async fn check(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        config: Value,
    ) -> Result<(), HostError> {
        self.check_as(ext, spec, &spec.id, Scope::Project, config)
            .await
    }

    /// [`Self::check`] as instance `id` of `scope` (its own credentials).
    async fn check_as(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        id: &str,
        scope: Scope,
        config: Value,
    ) -> Result<(), HostError> {
        let instance = self.instance(ext, spec, id, scope, config).await?;
        instance.start().await.map(|_| ())
    }

    async fn instance(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        id: &str,
        scope: Scope,
        config: Value,
    ) -> Result<Arc<Instance>, HostError> {
        let name = spec::instance_name(&ext.name, id);
        let declared = copy_approved(&self.deps, ext, spec).await?.declared;
        Ok(Arc::new(Instance {
            name,
            id: id.to_string(),
            scope,
            ext: ext.clone(),
            spec: spec.clone(),
            config,
            declared,
            deps: self.deps.clone(),
            registry: self.me.clone(),
            live: tokio::sync::Mutex::new(None),
            starting: tokio::sync::Mutex::new(()),
            not_before: parking_lot::Mutex::new(None),
            reading: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }))
    }

    /// Start `ext`'s provider `spec` with `config` — its default instance
    /// — and register what it declares ([`Self::enable_instance`]).
    pub async fn enable(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        config: Value,
    ) -> Result<(), HostError> {
        self.enable_instance(ext, spec, &spec.id, config).await
    }

    /// Start instance `id` of `ext`'s provider `spec` with `config` and
    /// register what it declares, under `id`: its commands' namespace and
    /// its capability provider. Consent, a matching handshake and a clean
    /// `check` come first: refused, nothing is registered (a failed start
    /// counts as a failure, [`crate::plugin_health`]).
    pub async fn enable_instance(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        id: &str,
        config: Value,
    ) -> Result<(), HostError> {
        self.enable_scoped(ext, spec, id, Scope::Project, config)
            .await
    }

    /// [`Self::enable_instance`] for an instance of `scope`.
    async fn enable_scoped(
        &self,
        ext: &Extension,
        spec: &ProviderSpec,
        id: &str,
        scope: Scope,
        config: Value,
    ) -> Result<(), HostError> {
        let name = spec::instance_name(&ext.name, id);
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
        let epoch = self.disable_epoch(&name);
        if let Some(owner) = bus.namespace_owner(id) {
            return Err(refuse(format!(
                "the command namespace `{id}` is already {owner}'s"
            )));
        }
        if self.work_items.get(id).is_ok() {
            return Err(refuse(format!("`{id}` is already a provider")));
        }
        let instance = match self.instance(ext, spec, id, scope, config).await {
            Ok(i) => i,
            Err(e) => {
                self.start_failed(&name, e.clone()).await;
                return Err(e);
            }
        };
        let vocabulary = bus.vocabulary().current();
        for t in &instance.declared.event_types {
            if vocabulary.schema(&t.event_type, t.v) != Some(&t.schema) {
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
                self.admit(&bus, instance, epoch).await.map_err(&refuse)?;
                {
                    let mut health = self.health.lock();
                    let h = health
                        .entry(name.clone())
                        .or_insert_with(|| InstanceHealth::new(InstanceState::Ready));
                    h.state = InstanceState::Ready;
                    h.consecutive_failures = 0;
                    h.last_ok_at = Some(now());
                }
                if let Err(e) = self.plugins.succeeded(&plugin_key(&name), None).await {
                    tracing::warn!(instance = %name, error = %e, "recording its health failed");
                }
                // Its items, before the first scheduled read (P7.A3) — in
                // the background: a large first read must not hold up the
                // Enable button or every other reconcile (tsk716).
                let me = self.me.clone();
                tokio::spawn(async move {
                    if let Some(registry) = me.upgrade() {
                        registry.sync_started(&name).await;
                    }
                });
                Ok(())
            }
            // It may come up: enabled and failing, its next call restarts
            // it with backoff.
            Err(e @ HostError::Failed { .. }) => {
                self.admit(&bus, instance, epoch).await.map_err(&refuse)?;
                self.failed(&name, e.to_string()).await;
                Ok(())
            }
            Err(e) => {
                self.start_failed(&name, e.clone()).await;
                Err(e)
            }
        }
    }

    /// How many times `instance` has been disabled.
    fn disable_epoch(&self, instance: &str) -> u64 {
        self.disables.lock().get(instance).copied().unwrap_or(0)
    }

    /// Register a started `instance` and count it running — unless it
    /// was disabled since its start began (`epoch`): the disable wins.
    /// Under the `running` lock, which a disable takes too.
    async fn admit(
        &self,
        bus: &Arc<CommandBus>,
        instance: Arc<Instance>,
        epoch: u64,
    ) -> Result<(), String> {
        let mut running = self.running.lock().await;
        if self.disable_epoch(&instance.name) != epoch {
            return Err(format!("`{}` was disabled while it started", instance.name));
        }
        self.register(bus, &instance)?;
        self.publish(&instance).await;
        running.insert(instance.name.clone(), instance);
        Ok(())
    }

    /// Its capability and features in `v_capability_provider`, while it
    /// runs: a work-items provider's as the host reads them (what the UI
    /// gates on), any other's as declared.
    async fn publish(&self, instance: &Instance) {
        let capability = &instance.spec.capability;
        let features = match self.work_items.get(&instance.id) {
            Ok(p) if capability == spec::WORK_ITEMS => {
                serde_json::to_value(p.features).unwrap_or(Value::Null)
            }
            _ => instance
                .declared
                .capabilities
                .iter()
                .find(|c| &c.capability == capability)
                .map(|c| c.features.clone())
                .unwrap_or(Value::Null),
        };
        let config = crate::config_service::read_config(&self.deps.config);
        let row = oxplow_db::CapabilityProvider {
            capability: capability.clone(),
            provider: instance.id.clone(),
            extension: Some(instance.ext.name.clone()),
            features,
            active: crate::capabilities::is_active(&config, capability, &instance.id),
        };
        if let Err(e) = oxplow_db::SqliteCapabilityStore::new(self.deps.db.clone())
            .upsert(row)
            .await
        {
            tracing::warn!(instance = %instance.name, error = %e, "publishing a provider's features failed");
        }
    }

    /// Put `instance`'s commands on the bus and its capability provider
    /// in its registry; all or nothing.
    fn register(&self, bus: &Arc<CommandBus>, instance: &Arc<Instance>) -> Result<(), String> {
        let id = &instance.id;
        bus.register_namespace(
            id,
            &format!("provider:{}", instance.name),
            commands(instance)?,
        )
        .map_err(|e| e.to_string())?;
        if instance.spec.capability == spec::WORK_ITEMS {
            match super::work_items::ExternalWorkItems::provider(instance) {
                Ok(provider) => self.work_items.register(provider),
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
        self.tear_down(running).await;
        true
    }

    /// Unregister a stopped instance and end its process.
    async fn tear_down(&self, running: Arc<Instance>) {
        if let Some(bus) = self.bus.upgrade() {
            bus.unregister_namespace(&running.id);
        }
        self.work_items.unregister(&running.id);
        if let Err(e) = oxplow_db::SqliteCapabilityStore::new(self.deps.db.clone())
            .remove(&running.spec.capability, &running.id)
            .await
        {
            tracing::warn!(instance = %running.name, error = %e, "withdrawing a provider's features failed");
        }
        running.live.lock().await.take();
    }

    /// A person approved program `<extension>/<provider id>` as it is
    /// now: every running instance of it restarts on what was approved
    /// (its declarations may have changed, and a start checks them against
    /// what it was enabled with).
    pub async fn approved(&self, program: &str) {
        let of_it: Vec<String> = self
            .running
            .lock()
            .await
            .values()
            .filter(|i| i.spec.approval_name(&i.ext.name) == program)
            .map(|i| i.name.clone())
            .collect();
        let mut stopped = false;
        for name in of_it {
            stopped |= self.stop(&name).await;
        }
        if stopped {
            self.reconcile().await;
        }
    }

    /// A person's Check / Enable / Disable on Settings → Integrations:
    /// write `extensionInstances.<instance>` through `config.set` and, to
    /// enable, run `plugin.enable`. Enabling checks first: an
    /// unapproved or unconfigured instance is refused and nothing is
    /// written.
    pub async fn set_instance(
        &self,
        actor: &Actor,
        instance: &str,
        enabled: bool,
        config: Value,
    ) -> Result<ProviderInstanceView, CommandError> {
        let Resolved {
            ext,
            spec,
            id,
            scope,
        } = self
            .resolve(instance)
            .map_err(|message| CommandError::Invalid {
                field: Some("/instance".into()),
                message,
            })?;
        // It is written where it lives: the project's entry (its own, or
        // its replacement of a global one), else the machine's file.
        let project = self.project_instances();
        let home = if scope == Scope::Global && !project.contains_key(instance) {
            Scope::Global
        } else {
            Scope::Project
        };
        if enabled {
            match self.check_as(&ext, &spec, &id, scope, config.clone()).await {
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
        // Enable (clearing an automatic disable) before writing: a failed
        // enable writes nothing, so the config never says enabled for an
        // instance that wasn't. The write then starts it (a reconcile).
        if enabled {
            let key = plugin_key(instance);
            bus.run(
                actor,
                crate::plugin_health::ENABLE,
                json!({ "plugin": key.plugin, "kind": key.kind, "contribution": key.contribution }),
                false,
            )
            .await?;
        }
        let mut all = match home {
            Scope::Project => project,
            Scope::Global => self.global_instances(),
        };
        let sync_minutes = all.get(instance).and_then(|c| c.sync_minutes);
        let provider = all.get(instance).and_then(|c| c.provider.clone());
        all.insert(
            instance.to_string(),
            oxplow_config::ExtensionInstanceConfig {
                enabled,
                config,
                sync_minutes,
                provider,
            },
        );
        self.write_instances(actor, home, all).await?;
        self.view(instance).await
    }

    /// A person adds another instance of `provider`: `instance`
    /// (`<extension>/<instance id>`), off and unconfigured until they set
    /// it up — the project's (written through `config.set`:
    /// `extensionInstances` is a person's key, so an agent's is refused or
    /// proposed) or, with `Scope::Global`, their own on this machine
    /// (`instances.yaml`, a person's only). Refused when
    /// the extension doesn't declare `provider`, or the id is taken — by
    /// another instance, or as another provider's own id.
    pub async fn add_instance(
        &self,
        actor: &Actor,
        instance: &str,
        provider: &str,
        scope: Scope,
    ) -> Result<ProviderInstanceView, CommandError> {
        let invalid = |message: String| CommandError::Invalid {
            field: Some("/instance".into()),
            message,
        };
        let Resolved { ext, id, .. } = self
            .resolve_as(instance, Some(provider))
            .map_err(&invalid)?;
        if self.instances_config().contains_key(instance) {
            return Err(invalid(format!("`{instance}` is already an instance")));
        }
        let mut all = match scope {
            Scope::Project => self.project_instances(),
            Scope::Global => self.global_instances(),
        };
        if id != provider && ext.providers.iter().any(|p| p.id == id) {
            return Err(invalid(format!(
                "`{id}` is already the id of one of `{}`'s providers (its default instance)",
                ext.name
            )));
        }
        all.insert(
            instance.to_string(),
            oxplow_config::ExtensionInstanceConfig {
                enabled: false,
                config: json!({}),
                sync_minutes: None,
                provider: (id != provider).then(|| provider.to_string()),
            },
        );
        self.write_instances(actor, scope, all).await?;
        self.view(instance).await
    }

    /// A person removes `instance`: it stops, its config entry and its
    /// credentials on this machine go. A project's replacement of a global
    /// instance is what goes first — the global one then shows through.
    pub async fn remove_instance(&self, actor: &Actor, instance: &str) -> Result<(), CommandError> {
        let resolved = self.resolve(instance).ok();
        let mut project = self.project_instances();
        let mut global = self.global_instances();
        // A project's replacement of a global one: the global one stays.
        let still_there = project.contains_key(instance) && global.contains_key(instance);
        let (home, all) = if project.remove(instance).is_some() {
            (Scope::Project, project)
        } else if global.remove(instance).is_some() {
            (Scope::Global, global)
        } else {
            return Err(CommandError::Invalid {
                field: Some("/instance".into()),
                message: format!("no provider instance `{instance}`"),
            });
        };
        self.write_instances(actor, home, all).await?;
        if still_there {
            return Ok(());
        }
        if let Some(Resolved {
            ext,
            spec,
            id,
            scope,
        }) = resolved
        {
            // A sign-in under way would store a token for what's gone.
            self.abandon_sign_ins(|(of, _)| of == instance).await;
            for name in &spec.credential_names() {
                let account = crate::collector_runner::instance_credential_account(
                    credential_scope(&self.deps, scope),
                    &ext.name,
                    &id,
                    name,
                );
                if let Err(e) = self.deps.secrets.delete(&account) {
                    tracing::warn!(%instance, credential = %name, error = %e, "removing an instance's credential failed");
                }
            }
        }
        self.health.lock().remove(instance);
        Ok(())
    }

    /// Write `scope`'s instances as `actor` and reconcile: the project's
    /// through `config.set` (`extensionInstances` is a person's key), the
    /// machine's file directly — a person's only: no command reaches it,
    /// so no agent or lens can.
    async fn write_instances(
        &self,
        actor: &Actor,
        scope: Scope,
        all: BTreeMap<String, oxplow_config::ExtensionInstanceConfig>,
    ) -> Result<(), CommandError> {
        match scope {
            Scope::Project => {
                let bus = self.bus.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "the command bus is gone".into(),
                })?;
                bus.run(
                    actor,
                    crate::commands::config_commands::SET,
                    json!({ "key": "extensionInstances", "value": all }),
                    matches!(actor, Actor::Human),
                )
                .await?;
            }
            Scope::Global => {
                if !matches!(actor, Actor::Human) {
                    return Err(CommandError::Denied {
                        reason: "this machine's global instances are a person's to change".into(),
                    });
                }
                let dir = self
                    .deps
                    .global_dir
                    .clone()
                    .ok_or_else(|| CommandError::Failed {
                        message: "this machine has no global config dir to keep instances in"
                            .into(),
                    })?;
                oxplow_config::GlobalInstances { instances: all }
                    .save(&dir)
                    .map_err(|e| CommandError::Failed {
                        message: e.to_string(),
                    })?;
            }
        }
        self.reconcile().await;
        Ok(())
    }

    /// `instance`'s declared credential `name`: its keychain account on
    /// this machine, how it is declared, and the instance it is of. The
    /// instance must exist: a credential is an instance's own.
    fn credential(
        &self,
        instance: &str,
        name: &str,
    ) -> Result<(String, spec::CredentialDecl, Resolved), DomainError> {
        let resolved = self.resolve(instance).map_err(DomainError::Invalid)?;
        let Resolved {
            ext,
            spec,
            id,
            scope,
        } = &resolved;
        let Some(decl) = spec.credentials.iter().find(|c| c.name == name).cloned() else {
            let names = spec.credential_names();
            return Err(DomainError::Invalid(format!(
                "provider `{}` declares no credential `{name}` (it declares: {})",
                spec.approval_name(&ext.name),
                if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join(", ")
                }
            )));
        };
        let account = crate::collector_runner::instance_credential_account(
            credential_scope(&self.deps, *scope),
            &ext.name,
            id,
            name,
        );
        Ok((account, decl, resolved))
    }

    /// Set (or, with `None`, forget) `instance`'s credential `name` in
    /// this machine's keychain. One the person signs in for is never
    /// given a value — forgetting it signs them out.
    pub fn set_credential(
        &self,
        instance: &str,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), DomainError> {
        let (account, decl, _) = self.credential(instance, name)?;
        if decl.oauth.is_some() && value.is_some() {
            return Err(DomainError::Invalid(format!(
                "`{name}` is a credential you sign in for (Settings → Integrations → Sign \
                 in); it isn't given a value"
            )));
        }
        match value {
            Some(v) => self.deps.secrets.set(&account, v),
            None => self.deps.secrets.delete(&account),
        }
        .map_err(|e| DomainError::Storage(format!("credential `{name}`: {e}")))
    }

    /// `instance`'s credential changed: it restarts on the new value (a
    /// running one), or starts if the credential was what it lacked.
    pub async fn credential_changed(&self, instance: &str) {
        self.stop(instance).await;
        self.reconcile().await;
    }

    /// Start signing in for `instance`'s credential `name` (one declared
    /// with `oauth:`): where the person goes to do it. When they have, the
    /// token is kept in the keychain, the instance restarts on it, and the
    /// renderer hears `CredentialChanged` — with why, when it came to
    /// nothing. A sign-in already under way for it is abandoned.
    pub async fn begin_sign_in(&self, instance: &str, name: &str) -> Result<String, DomainError> {
        let (account, decl, resolved) = self.credential(instance, name)?;
        let Some(oauth_decl) = decl.oauth else {
            return Err(DomainError::Invalid(format!(
                "`{name}` isn't a credential you sign in for; paste its value instead"
            )));
        };
        // Where it signs in, and where the code, the verifier and the
        // client secret go, are part of what a person approved: only as
        // they were approved (tsk824). The declaration used below is the
        // one this checks.
        if !crate::exec_consent::may_run_provider(
            &self.deps.approvals,
            &self.deps.project_dir,
            &resolved.ext,
            &resolved.spec,
        ) {
            return Err(DomainError::Invalid(format!(
                "provider `{}` isn't approved as it is now (where `{name}` signs in is part of \
                 what you approve): approve it on Settings → Data → Programs first",
                resolved.spec.approval_name(&resolved.ext.name)
            )));
        }
        let client_secret = match &oauth_decl.client_secret {
            None => None,
            Some(secret) => {
                let of_secret = crate::collector_runner::instance_credential_account(
                    credential_scope(&self.deps, resolved.scope),
                    &resolved.ext.name,
                    &resolved.id,
                    secret,
                );
                match self
                    .deps
                    .secrets
                    .get(&of_secret)
                    .map_err(|e| DomainError::Storage(format!("credential `{secret}`: {e}")))?
                {
                    Some(value) => Some(value),
                    None => {
                        return Err(DomainError::Invalid(format!(
                            "set `{secret}` first: `{name}`'s sign-in needs it"
                        )))
                    }
                }
            }
        };
        let key = (instance.to_string(), name.to_string());
        // Before the new one listens: a fixed `redirect_port` is the old
        // one's until it stops.
        self.abandon_sign_ins(|k| *k == key).await;
        let sign_in = oauth::begin(&oauth_decl, client_secret)
            .await
            .map_err(DomainError::Invalid)?;
        let (me, deps) = (self.me.clone(), self.deps.clone());
        let (instance, name) = key.clone();
        let done = sign_in.done;
        let waiting = tokio::spawn(async move {
            let outcome = match done.await {
                Ok(token) => oauth::store(deps.secrets.as_ref(), &account, &token),
                Err(why) => Err(why),
            };
            if let Some(registry) = me.upgrade() {
                registry
                    .sign_ins
                    .lock()
                    .remove(&(instance.clone(), name.clone()));
                if outcome.is_ok() {
                    registry.credential_changed(&instance).await;
                }
            }
            deps.events
                .emit(crate::events::OxplowEvent::CredentialChanged {
                    instance,
                    name,
                    error: outcome.err(),
                });
        });
        self.sign_ins.lock().insert(key, waiting);
        Ok(sign_in.authorize_url)
    }

    /// Stop waiting for the sign-ins `which` picks: each stops listening
    /// before this returns (its port is free), and stores nothing.
    async fn abandon_sign_ins(&self, which: impl Fn(&(String, String)) -> bool) {
        let abandoned: Vec<_> = {
            let mut sign_ins = self.sign_ins.lock();
            let keys: Vec<_> = sign_ins.keys().filter(|k| which(k)).cloned().collect();
            keys.iter().filter_map(|k| sign_ins.remove(k)).collect()
        };
        for waiting in abandoned {
            waiting.abort();
            // Ended (or cancelled): its listener is dropped with it.
            let _ = waiting.await;
        }
    }

    /// Check `instance` with `config` for a person (Settings' Check): its
    /// view with the outcome as its state; nothing is enabled or written.
    pub async fn check_instance(
        &self,
        instance: &str,
        config: Value,
    ) -> Result<ProviderInstanceView, CommandError> {
        let Resolved {
            ext,
            spec,
            id,
            scope,
        } = self
            .resolve(instance)
            .map_err(|message| CommandError::Invalid {
                field: Some("/instance".into()),
                message,
            })?;
        let mut view = self.view(instance).await?;
        view.health.state = match self.check_as(&ext, &spec, &id, scope, config).await {
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

    /// A person enabled `instance` again (`plugin.enable`): its failure
    /// count and backoff start over.
    pub(crate) async fn reset(&self, instance: &str) {
        if let Some(h) = self.health.lock().get_mut(instance) {
            h.consecutive_failures = 0;
        }
        if let Some(i) = self.get(instance).await {
            *i.not_before.lock() = None;
        }
    }

    /// One instance's view.
    pub async fn view(&self, instance: &str) -> Result<ProviderInstanceView, CommandError> {
        self.list()
            .await
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

    /// A start or call failed: count it ([`crate::plugin_health`]); the
    /// verdict either backs the next start off or stops the instance.
    pub(super) async fn failed(&self, instance: &str, error: String) {
        let verdict = match self.plugins.failed(&plugin_key(instance), &error).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(instance, error = %e, "recording a provider failure failed");
                crate::plugin_health::Verdict::Backoff { failures: 1 }
            }
        };
        let failures = {
            let mut health = self.health.lock();
            let h = health
                .entry(instance.to_string())
                .or_insert_with(|| InstanceHealth::new(InstanceState::Checking));
            let mut errors = match &h.state {
                InstanceState::Failing { errors } => errors.clone(),
                _ => Vec::new(),
            };
            errors.push(error.clone());
            let excess = errors.len().saturating_sub(ERRORS_KEPT);
            errors.drain(..excess);
            h.state = InstanceState::Failing { errors };
            if let crate::plugin_health::Verdict::Backoff { failures } = verdict {
                h.consecutive_failures = u32::try_from(failures).unwrap_or(u32::MAX);
            }
            h.consecutive_failures
        };
        match verdict {
            crate::plugin_health::Verdict::Disabled { reason } => self.halt(instance, reason).await,
            crate::plugin_health::Verdict::Backoff { .. } => {
                if let Some(i) = self.get(instance).await {
                    let wait = self
                        .deps
                        .backoff
                        .saturating_mul(1 << failures.saturating_sub(1).min(6))
                        .min(MAX_BACKOFF);
                    *i.not_before.lock() = Some(Instant::now() + wait);
                }
            }
        }
    }

    /// Stop `instance` and keep it off on this machine until a person
    /// enables it, for `reason` (`plugin.disabled@1`).
    pub(super) async fn disable(&self, instance: &str, reason: String) {
        if let Err(e) = self.plugins.disable(&plugin_key(instance), &reason).await {
            tracing::error!(instance, error = %e, "recording the disable failed");
        }
        self.halt(instance, reason).await;
    }

    /// Stop a disabled `instance` and show why.
    async fn halt(&self, instance: &str, reason: String) {
        let removed = {
            let mut running = self.running.lock().await;
            *self
                .disables
                .lock()
                .entry(instance.to_string())
                .or_default() += 1;
            running.remove(instance)
        };
        if let Some(running) = removed {
            self.tear_down(running).await;
        }
        self.set_state(instance, InstanceState::Disabled { reason });
    }

    /// Its service said to wait (`wait`, when it said how long).
    pub(super) fn note_rate_limit(&self, instance: &str, wait: Option<Duration>) {
        let until = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms()
                + wait.unwrap_or(RATE_LIMIT_WAIT_MAX).as_millis() as i64,
        );
        if let Some(h) = self.health.lock().get_mut(instance) {
            h.rate_limited_until = Some(until.to_string());
        }
    }

    /// What a read in progress says it's doing; `None` when it ends.
    pub(super) fn set_activity(&self, instance: &str, activity: Option<String>) {
        if let Some(h) = self.health.lock().get_mut(instance) {
            h.activity = activity;
        }
    }

    /// Whether its service asked it to wait past now.
    pub(super) fn is_rate_limited(&self, instance: &str) -> bool {
        let now = oxplow_domain::Timestamp::now().unix_ms();
        self.health
            .lock()
            .get(instance)
            .and_then(|h| h.rate_limited_until.as_deref())
            .and_then(|t| oxplow_domain::Timestamp::parse(t).ok())
            .is_some_and(|t| t.unix_ms() > now)
    }

    /// Whether `instance` is the one running under its name: a stopped
    /// instance's late results — and those of the one a restart replaced —
    /// aren't the running one's (tsk820).
    async fn is_current(&self, instance: &Instance) -> bool {
        self.running
            .lock()
            .await
            .get(&instance.name)
            .is_some_and(|running| std::ptr::eq(Arc::as_ptr(running), instance))
    }

    /// A call `instance` made failed: counted like any failure, while it
    /// is the running instance. A call cut short by its own stop isn't one.
    pub(super) async fn call_failed(&self, instance: &Instance, error: String) {
        if self.is_current(instance).await {
            self.failed(&instance.name, error).await;
        }
    }

    /// A call `instance` made succeeded: its health says so, while it is
    /// the running instance.
    pub(super) async fn call_succeeded(&self, instance: &Instance, took: Duration) {
        if !self.is_current(instance).await {
            return;
        }
        let made_by = instance;
        let instance = instance.name.as_str();
        if let Err(e) = self
            .plugins
            .succeeded(&plugin_key(instance), Some(took))
            .await
        {
            tracing::warn!(instance, error = %e, "recording its health failed");
        }
        let ms = took.as_secs_f64() * 1000.0;
        // Under the lock a stop takes: it either sees this state and
        // replaces it, or has already removed the instance.
        let running = self.running.lock().await;
        if !running
            .get(instance)
            .is_some_and(|r| std::ptr::eq(Arc::as_ptr(r), made_by))
        {
            return;
        }
        let mut health = self.health.lock();
        let h = health
            .entry(instance.to_string())
            .or_insert_with(|| InstanceHealth::new(InstanceState::Ready));
        h.state = InstanceState::Ready;
        h.consecutive_failures = 0;
        h.last_ok_at = Some(now());
        h.rate_limited_until = None;
        h.mean_invoke_ms = Some(match h.mean_invoke_ms {
            None => ms,
            Some(mean) => mean + (ms - mean) / 10.0,
        });
    }
}

/// A typed request that is cancelled (`$/cancel`) and fails if no reply
/// comes within `limit`.
async fn call_within<P: Serialize, R: serde::de::DeserializeOwned>(
    peer: &oxplow_provider_protocol::Peer,
    method: &str,
    params: &P,
    limit: Duration,
) -> Result<R, ProtocolError> {
    let params =
        serde_json::to_value(params).map_err(|e| ProtocolError::InvalidParams(e.to_string()))?;
    let call = peer.start(method, params).await?;
    let id = call.id;
    match tokio::time::timeout(limit, call.reply()).await {
        Ok(reply) => serde_json::from_value(reply?)
            .map_err(|e| ProtocolError::Internal(format!("`{method}` result: {e}"))),
        Err(_) => {
            let _ = peer.cancel(id).await;
            Err(ProtocolError::Internal(format!(
                "`{method}` timed out after {}s",
                limit.as_secs_f32()
            )))
        }
    }
}

/// [`host::approved_copy`] off the runtime (it copies and hashes files).
async fn copy_approved(
    deps: &HostDeps,
    ext: &Extension,
    spec: &ProviderSpec,
) -> Result<host::ApprovedCopy, HostError> {
    let (project, copies, approvals) = (
        deps.project_dir.clone(),
        deps.copies.clone(),
        deps.approvals.clone(),
    );
    let (ext, spec) = (ext.clone(), spec.clone());
    let name = spec.approval_name(&ext.name);
    tokio::task::spawn_blocking(move || {
        host::approved_copy(&project, &copies, &approvals, &ext, &spec)
    })
    .await
    .map_err(|e| HostError::Failed {
        name,
        message: format!("copying it to run: {e}"),
    })?
}

fn now() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The instance's declared commands, as bus commands — less its
/// capability's verbs, which run as `work_item.<verb>` (one write surface,
/// dispatched by ref), never as `<id>.<verb>`.
fn commands(instance: &Arc<Instance>) -> Result<Vec<Command>, String> {
    let id = instance.id.clone();
    let verbs: &[&str] = if instance.spec.capability == spec::WORK_ITEMS {
        &oxplow_domain::work_items::VERBS
    } else {
        &[]
    };
    instance
        .declared
        .commands
        .iter()
        .filter(|decl| !verbs.contains(&decl.name.as_str()))
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

/// Keep the instances matching the extensions: once at boot, then
/// whenever the primary worktree's extensions may have changed (the
/// extension catalog's signal; an instance's own config arrives through
/// the `config.providers` reactor).
pub fn spawn_reconciler(state: Arc<crate::Services>) {
    let mut changes = state.extension_catalog.changes();
    tokio::spawn(async move {
        state.providers.reconcile().await;
        // Lagging only means it missed some: one pass covers them.
        while let Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
            changes.recv().await
        {
            state.providers.reconcile().await;
        }
    });
}
