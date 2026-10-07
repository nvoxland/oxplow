//! The provider instances (`Services.providers`): one long-lived process
//! per enabled instance, its health, and restart with backoff.
//!
//! An instance is `<extension>/<instance id>`, configured in the
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
//! `plugin_health` says so — until a person runs `oxplow.plugin.enable`.

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
use crate::commands::{Command, CommandBus, Handler, HandlerOutput, Invocation};
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

/// How many failed writes' keys an instance remembers, so a write's
/// re-sends count once toward its health.
const FAILED_KEYS_KEPT: usize = 256;

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
    /// Every capability's implementations: an instance's while it runs.
    pub capabilities: Arc<crate::capabilities::CapabilityRegistry>,
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
    /// Each global instance's credential count
    /// ([`oxplow_config::CredentialGenerations`]) as this oxplow last
    /// started it (tsk842).
    credentials_seen: BTreeMap<String, u64>,
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
    /// Enabled, but oxplow won't run it as configured: its id is a
    /// command namespace or provider something else already has, or it
    /// declares an event type oxplow doesn't know with that schema.
    /// `reason` says which (tsk840).
    Refused {
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
    /// For one signed in for at a service with its redirect port
    /// registered (`redirect_port`): the loopback port the shell must
    /// catch the redirect on. Any free port otherwise.
    pub redirect_port: Option<u16>,
}

/// One `(instance, credential)`'s sign-in gate.
type SignInGate = Arc<tokio::sync::Mutex<()>>;

/// A sign-in's number, from its begin: the renderer tells its own
/// sign-in's news from another's by it, and cancels it by it (tsk929).
pub type SignInId = u32;

/// A sign-in begun: the page the person signs in on, and its number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BegunSignIn {
    pub url: String,
    pub sign_in: SignInId,
}

/// A sign-in waiting for its redirect.
struct SignInUnderWay {
    /// Which start it was ([`ProviderRegistry`]'s `sign_in_seq`): the
    /// `sign_in` its news names, and what cancels it.
    seq: SignInId,
    /// The keychain account its token goes to.
    account: String,
    pending: oauth::PendingSignIn,
    /// Ends it unfinished after [`oauth::SIGN_IN_WAIT`].
    expiry: tokio::task::JoinHandle<()>,
}

/// How handing a redirect to a sign-in went (`complete_oauth_sign_in`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SignInCompletion {
    /// Signed in: the token is kept and the instance restarts on it.
    SignedIn,
    /// The sign-in ended without a token: why (what `CredentialChanged`
    /// carries too).
    Failed { error: String },
    /// Not this sign-in's redirect (another `state`, another path):
    /// nothing was done, and the sign-in still waits.
    NotThisSignIn { reason: String },
}

/// An instance as Settings → Integrations shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstanceView {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// The project's, or the person's on this machine (every project).
    /// A project's entry is the project's, config and credentials, even
    /// where it replaces a global one of the same name (tsk838).
    pub scope: Scope,
    /// This project's own entry replaces a global instance of the same
    /// name here.
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

/// Why a call is tried again on a renewed sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Its service refused its credentials (`Auth`), naming the one it
    /// refused when it said.
    Auth(Option<String>),
    /// It was cut off because another caller's refusal renewed a sign-in
    /// and ended the process under it: nothing to renew, only retry.
    RenewedUnder,
}

/// A started process and the handle its `check` returned.
/// A start's `check` answered `Auth`: the credential it named (if any),
/// and the credentials the process was given.
type CheckRefusal = (Option<String>, BTreeMap<String, String>);

struct Live {
    conn: Connection,
    handle: Handle,
    /// When it started: a call older than it wasn't refused by it.
    since: Instant,
    /// The credentials it was given: a refusal is of one of these (tsk928).
    given: BTreeMap<String, String>,
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
    /// Stopped (torn down): it never starts again, so a late caller still
    /// holding it can't bring back a process nobody runs (tsk836).
    stopped: std::sync::atomic::AtomicBool,
    /// One start at a time; held across a start, which `live` never is.
    starting: tokio::sync::Mutex<()>,
    /// When each signed-in credential was last renewed because its
    /// service refused it (tsk828, per credential since tsk821).
    renewed_at: parking_lot::Mutex<BTreeMap<String, Instant>>,
    /// The idempotency keys of writes whose failure its health has counted
    /// (newest last, at most [`FAILED_KEYS_KEPT`]): a write sent again
    /// under its key — an automatic retry, a person's — failing again is
    /// the same failure, counted once (tsk913). A key that lands is
    /// forgotten.
    failed_keys: parking_lot::Mutex<std::collections::VecDeque<String>>,
    not_before: parking_lot::Mutex<Option<Instant>>,
    /// One read per collector at a time (tsk715): a second waits, then
    /// resumes from the checkpoint the first left.
    pub(super) reading:
        parking_lot::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Instance {
    /// Whether it keeps `idempotent_writes` (its declarations): a write
    /// sent again with its key is done once.
    fn idempotent_writes(&self) -> bool {
        self.declared
            .capabilities
            .iter()
            .any(|c| c.features.get("idempotent_writes").and_then(Value::as_bool) == Some(true))
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
    /// Its credentials' values, for a start. `refused` names the signed-in
    /// credential an `Auth` refused and the token it refused: that one is
    /// renewed first, unless it was replaced meanwhile (tsk928).
    async fn credentials(
        &self,
        refused: Option<(&str, &str)>,
    ) -> Result<BTreeMap<String, String>, HostError> {
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
                refused.filter(|(n, _)| *n == name).map(|(_, token)| token),
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
    /// credentials (`Auth`) is tried once more on a renewed token, when
    /// the refusal points at a signed-in credential ([`Self::renewable`]).
    async fn start(&self) -> Result<Live, HostError> {
        match self.start_once(None).await {
            Err((e, Some((named, given)))) => match self.renewable(named.as_deref()) {
                Some(cred) => match given.get(&cred) {
                    Some(token) => self
                        .start_once(Some((&cred, token)))
                        .await
                        .map_err(|(e, _)| e),
                    None => Err(e),
                },
                None => Err(e),
            },
            other => other.map_err(|(e, _)| e),
        }
    }

    /// One start, renewing the `refused` credential's token first; the
    /// error carries, when its `check` answered `Auth`, the refusal.
    async fn start_once(
        &self,
        refused: Option<(&str, &str)>,
    ) -> Result<Live, (HostError, Option<CheckRefusal>)> {
        let plain = |e: HostError| (e, None);
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
        let credentials = self.credentials(refused).await.map_err(plain)?;
        let names: Vec<String> = credentials.keys().cloned().collect();
        let given = credentials.clone();
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
            let auth = match &e {
                ProtocolError::Auth { credential, .. } => Some((credential.clone(), given.clone())),
                _ => None,
            };
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
                given,
            }),
            _ => Err(plain(HostError::Unconfigured {
                name: self.name.clone(),
                problems: checked.problems,
            })),
        }
    }

    /// The signed-in credential a refusal points at (tsk821): the one an
    /// `Auth` names, when it signs in for that one; with none named, its
    /// credential when it is handed exactly one and signs in for it — with
    /// two or more (signed in or pasted) it can't know which was refused,
    /// and renews none, so a good sign-in is never lapsed for a pasted
    /// key's refusal (tsk908). A named credential that isn't signed in for
    /// (a pasted key) has nothing to renew.
    pub(super) fn renewable(&self, named: Option<&str>) -> Option<String> {
        let credentials = &self.spec.credentials;
        match named {
            Some(name) => credentials
                .iter()
                .find(|c| c.oauth.is_some() && c.name == name)
                .map(|c| c.name.clone()),
            None => match credentials.as_slice() {
                [only] if only.oauth.is_some() => Some(only.name.clone()),
                _ => None,
            },
        }
    }

    /// Whether any of its sign-ins was renewed (and its process ended for
    /// that) after `t`.
    pub(super) fn renewed_since(&self, t: Instant) -> bool {
        self.renewed_at.lock().values().any(|at| *at > t)
    }

    /// A call made at `called` was refused: renew what `refusal` points at
    /// and end the process, so the next call starts on it. Whether to try
    /// the call once more — false when there is nothing to renew. A
    /// credential renewed since `called` (another caller refused at the
    /// same time got here first), or a process started since on newer
    /// tokens, needs nothing but the retry (tsk828).
    pub(super) async fn reauthorize(&self, called: Instant, refusal: &Refusal) -> bool {
        let cred = match refusal {
            Refusal::RenewedUnder => return true,
            Refusal::Auth(named) => match self.renewable(named.as_deref()) {
                Some(cred) => cred,
                None => return false,
            },
        };
        let _one = self.starting.lock().await;
        if self
            .renewed_at
            .lock()
            .get(&cred)
            .is_some_and(|at| *at > called)
        {
            return true;
        }
        // The token its process was given is the one refused.
        let refused = {
            let mut live = self.live.lock().await;
            if live.as_ref().is_some_and(|l| l.since > called) {
                return true;
            }
            live.take().and_then(|l| l.given.get(&cred).cloned())
        };
        self.renewed_at.lock().insert(cred.clone(), Instant::now());
        // What can't be renewed shows when it next starts (its `check`
        // names the credential); here the only question is whether to
        // try again.
        let Some(refused) = refused else {
            return true;
        };
        if let Err(e) = self.credentials(Some((&cred, &refused))).await {
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
        let stopped = || CommandError::Failed {
            message: format!("provider `{}` was stopped", self.name),
        };
        if self.is_stopped() {
            return Err(stopped());
        }
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
        if self.is_stopped() {
            return Err(stopped());
        }
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
                let mut live = self.live.lock().await;
                // Stopped while it started: the new process goes with it
                // (a stop marks it stopped before it takes `live`).
                if self.is_stopped() {
                    return Err(stopped());
                }
                *self.not_before.lock() = None;
                let out = (l.conn.peer.clone(), l.handle.clone());
                *live = Some(l);
                Ok(out)
            }
            Err(e) => {
                let message = e.to_string();
                if let Some(r) = self.registry.upgrade() {
                    r.start_failed(&self.name, Some(self), e).await;
                }
                Err(CommandError::Failed { message })
            }
        }
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether it has a process.
    #[cfg(test)]
    pub(super) async fn has_process(&self) -> bool {
        self.live.lock().await.is_some()
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

    /// End its process; the next call starts a new one.
    pub(crate) async fn end_process(&self) {
        self.live.lock().await.take();
    }

    /// Forget the process `peer` talks to when it died under a call, so
    /// the next call restarts it.
    pub(super) async fn forget_if_closed(&self, peer: &oxplow_provider_protocol::Peer) {
        if peer.is_closed() {
            self.live.lock().await.take();
        }
    }

    /// Run one of its declared commands.
    /// Run `command`. `idempotency_key` is the write's key (an effect's
    /// step's); without one a key is minted here. Either way the same key
    /// goes with every re-send below, so a provider that keeps
    /// `idempotent_writes` does the write once. A call refused (`Auth`,
    /// `RateLimited`) never landed and is sent again; one cut off under
    /// way (its process ended for another caller's renewal) may have
    /// landed, and is sent again only to a provider that keeps the
    /// promise.
    pub async fn invoke(
        &self,
        command: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<InvokeResult, CommandError> {
        let key =
            idempotency_key.unwrap_or_else(|| format!("call:{}", uuid::Uuid::new_v4().simple()));
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
                    idempotency_key: Some(key.clone()),
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
            // Its service refused its credentials — or the process was
            // ended under this call to renew them for another caller: once,
            // on renewed ones.
            let refusal = match &result {
                Err(ProtocolError::Auth { credential, .. }) => {
                    Some(Refusal::Auth(credential.clone()))
                }
                Err(_)
                    if peer.is_closed()
                        && self.renewed_since(started)
                        && self.idempotent_writes() =>
                {
                    Some(Refusal::RenewedUnder)
                }
                _ => None,
            };
            if let Some(refusal) = refusal {
                if !reauthorized && self.reauthorize(started, &refusal).await {
                    reauthorized = true;
                    continue;
                }
            }
            return self.after_call(result, started, &key).await;
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
            r.note_rate_limit(self, wait).await;
        }
        match wait {
            Some(wait) if !retried && wait <= RATE_LIMIT_WAIT_MAX => {
                tokio::time::sleep(wait).await;
                None
            }
            _ => Some(CommandError::Unavailable {
                message: format!(
                    "provider `{}` is rate limited ({message}){}",
                    self.name,
                    match wait {
                        Some(w) => format!("; try again in {}s", w.as_secs().max(1)),
                        None => String::new(),
                    }
                ),
                retry_after_ms,
            }),
        }
    }

    async fn after_call(
        &self,
        result: Result<InvokeResult, ProtocolError>,
        started: Instant,
        key: &str,
    ) -> Result<InvokeResult, CommandError> {
        let registry = self.registry.upgrade();
        match result {
            Ok(out) => {
                self.failed_keys.lock().retain(|k| k != key);
                if let Some(r) = registry {
                    r.call_succeeded(self, started.elapsed()).await;
                }
                Ok(out)
            }
            Err(e) => {
                // A refused input or a cancel is the caller's, not a
                // failure; a write that already failed under its key is
                // that same failure (tsk913).
                let counts = !matches!(
                    e,
                    ProtocolError::InvalidInput { .. } | ProtocolError::Cancelled
                ) && self.first_failure_of(key);
                let err = self.command_error(e);
                if let (true, Some(r)) = (counts, registry) {
                    r.call_failed(self, err.to_string()).await;
                }
                Err(err)
            }
        }
    }

    /// A provider's error as the bus's: a refused input is the caller's;
    /// one its service may get past (it erred, the call timed out or its
    /// process died — `Internal`) is `Unavailable`, worth sending again;
    /// anything else — refused credentials renewal didn't fix, a method or
    /// configuration it lacks, a cancel — is `Failed`, a person's (tsk914).
    /// Whether this is the first counted failure of the write `key`;
    /// remembers it.
    fn first_failure_of(&self, key: &str) -> bool {
        let mut failed = self.failed_keys.lock();
        if failed.iter().any(|k| k == key) {
            return false;
        }
        if failed.len() == FAILED_KEYS_KEPT {
            failed.pop_front();
        }
        failed.push_back(key.to_string());
        true
    }

    pub(super) fn command_error(&self, e: ProtocolError) -> CommandError {
        match e {
            ProtocolError::InvalidInput { field, message } => CommandError::Invalid {
                field: Some(field),
                message: format!("{}: {message}", self.id),
            },
            ProtocolError::Internal(message) => CommandError::Unavailable {
                message: format!("provider `{}`: {message}", self.name),
                retry_after_ms: None,
            },
            other => CommandError::Failed {
                message: format!("provider `{}`: {other}", self.name),
            },
        }
    }

    /// A returned event as the envelope the bus logs: a type it declared,
    /// and (for a work-item record) an item of its own. `filed_on` is the
    /// thread a new item was filed on (`oxplow.work_item.create`'s, tsk1058).
    pub(crate) fn envelope(
        &self,
        actor: &Actor,
        filed_on: Option<oxplow_domain::ThreadId>,
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
        // The thread an item was filed on anchors its record, so it keeps
        // that thread (tsk1041, tsk1058); otherwise an agent's thread
        // anchors what it did.
        let mut anchors = match actor {
            Actor::Agent {
                thread_id,
                stream_id,
            } => oxplow_domain::Anchors {
                thread_id: *thread_id,
                stream_id: *stream_id,
                ..Default::default()
            },
            _ => oxplow_domain::Anchors::default(),
        };
        if filed_on.is_some() {
            anchors.thread_id = filed_on;
        }
        Envelope::new(draft.event_type, draft.v, actor.source(), draft.payload)
            .map(|e| e.with_subject(draft.subject).with_anchors(anchors))
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
    /// Set once its first reconcile has run: what waits for the
    /// instances the config names to be up (tsk915).
    reconciled: tokio::sync::watch::Sender<bool>,
    /// How many times each instance has been disabled: a start that began
    /// before a disable doesn't register what the disable stopped.
    disables: parking_lot::Mutex<BTreeMap<String, u64>>,
    /// The failure policy instances share with every plugin contribution.
    pub(super) plugins: crate::plugin_health::PluginHealth,
    /// This machine's global instances, re-read when their file changes.
    global: parking_lot::Mutex<GlobalFile>,
    /// Sign-ins under way, by `(instance, credential)`: a newer one for
    /// the same credential replaces the older.
    sign_ins: parking_lot::Mutex<BTreeMap<(String, String), SignInUnderWay>>,
    /// One gate per `(instance, credential)`, held while that sign-in is
    /// started, finished, expired or abandoned — across a finish's code
    /// exchange, so an abandon (a removed instance) waits for it and
    /// nothing is kept for what's gone (tsk826) — and only for it: a slow
    /// token endpoint holds up no other sign-in (tsk910).
    sign_in_gates: parking_lot::Mutex<BTreeMap<(String, String), SignInGate>>,
    /// One change to a person's instances at a time in this process: each
    /// reads them as they are and writes them back before another reads
    /// (tsk837). Never held across a check or a reconcile.
    instances_gate: tokio::sync::Mutex<()>,
    /// Numbers each sign-in, so a finished one untracks only itself.
    sign_in_seq: std::sync::atomic::AtomicU32,
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

/// What a removed instance was: enough to find what it left behind.
struct Gone {
    ext: String,
    id: String,
    /// The provider it was an instance of.
    provider: String,
    /// Its credentials' names, when its extension still declares it.
    credentials: Option<Vec<String>>,
}

impl Gone {
    fn of(
        instance: &str,
        resolved: Option<Resolved>,
        configured: Option<&oxplow_config::ExtensionInstanceConfig>,
    ) -> Gone {
        match resolved {
            Some(r) => Gone {
                ext: r.ext.name.clone(),
                id: r.id.clone(),
                provider: r.spec.id.clone(),
                credentials: Some(r.spec.credential_names()),
            },
            None => {
                let (ext, id) = instance.split_once('/').unwrap_or((instance, ""));
                Gone {
                    ext: ext.to_string(),
                    id: id.to_string(),
                    provider: configured
                        .and_then(|c| c.provider.clone())
                        .unwrap_or_else(|| id.to_string()),
                    credentials: None,
                }
            }
        }
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
        // What this oxplow starts on: a credential change counted later is
        // one it hasn't started on.
        let global = GlobalFile {
            credentials_seen: deps
                .global_dir
                .as_deref()
                .map(oxplow_config::CredentialGenerations::load)
                .unwrap_or_default(),
            ..GlobalFile::default()
        };
        Arc::new_cyclic(|me| Self {
            plugins,
            deps,
            me: me.clone(),
            bus: Arc::downgrade(bus),
            work_items,
            running: tokio::sync::Mutex::new(BTreeMap::new()),
            health: parking_lot::Mutex::new(BTreeMap::new()),
            reconciling: tokio::sync::Mutex::new(()),
            reconciled: tokio::sync::watch::Sender::new(false),
            disables: parking_lot::Mutex::new(BTreeMap::new()),
            global: parking_lot::Mutex::new(global),
            sign_ins: parking_lot::Mutex::new(BTreeMap::new()),
            sign_in_gates: parking_lot::Mutex::new(BTreeMap::new()),
            instances_gate: tokio::sync::Mutex::new(()),
            sign_in_seq: std::sync::atomic::AtomicU32::new(1),
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
    /// one here. A project's entry is the project's — its config and its
    /// credentials — whatever the person has globally, so a global one of
    /// the same name appearing or going never moves it onto other
    /// credentials (tsk838). An instance configured nowhere is the
    /// project's too.
    fn scope_of(&self, instance: &str) -> (Scope, bool) {
        let global = self.global_instances().contains_key(instance);
        if self.project_instances().contains_key(instance) {
            (Scope::Project, global)
        } else if global {
            (Scope::Global, false)
        } else {
            (Scope::Project, false)
        }
    }

    /// The machine's instances file changed since the last reconcile
    /// (another project's oxplow wrote it): reconcile. Called on the sync
    /// timer; `true` when it did.
    pub async fn reconcile_if_global_changed(&self) -> bool {
        // A global instance whose credentials another oxplow changed
        // restarts on them (tsk842): their values are in the keychain,
        // which nothing here can watch, so the other bumped a count.
        let changed: Vec<String> = match self.deps.global_dir.as_ref() {
            None => Vec::new(),
            Some(dir) => {
                let now = oxplow_config::CredentialGenerations::load(dir);
                let mut file = self.global.lock();
                let changed = now
                    .iter()
                    .filter(|(name, n)| file.credentials_seen.get(*name) != Some(n))
                    .map(|(name, _)| name.clone())
                    .collect();
                file.credentials_seen = now;
                changed
            }
        };
        let mut restarted = false;
        for instance in changed {
            if self.scope_of(&instance).0 == Scope::Global {
                restarted |= self.stop(&instance).await;
            }
        }
        if !restarted && self.global.lock().reconciled_at == self.global_mtime() {
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
                        redirect_port: c.oauth.as_ref().and_then(|o| o.redirect_port),
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
                    // A global instance of an extension this project
                    // doesn't have is the person's elsewhere: not this
                    // project's to list. One of an extension it has that
                    // doesn't resolve is listed, so it can be seen and
                    // removed (tsk841).
                    let (scope, overridden) = self.scope_of(instance);
                    let here = self
                        .extensions()
                        .iter()
                        .any(|e| e.enabled && e.name == extension);
                    if scope == Scope::Global && !here {
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
        self.reconcile_pass().await;
        self.reconciled.send_replace(true);
    }

    /// Once its first reconcile has run — at once after that.
    pub async fn first_reconciled(&self) {
        let mut done = self.reconciled.subscribe();
        let _ = done.wait_for(|d| *d).await;
    }

    async fn reconcile_pass(&self) {
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
        // The config may name another active implementation.
        let config = crate::config_service::read_config(&self.deps.config);
        if let Err(e) = self.deps.capabilities.publish(&config, &self.deps.db).await {
            tracing::warn!(error = %e, "restating the active implementations failed");
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
            stopped: std::sync::atomic::AtomicBool::new(false),
            starting: tokio::sync::Mutex::new(()),
            renewed_at: parking_lot::Mutex::new(BTreeMap::new()),
            failed_keys: parking_lot::Mutex::new(std::collections::VecDeque::new()),
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
        // What it is as configured can't run: its row says why (tsk840).
        let refused = |reason: String| {
            self.set_state(
                &name,
                InstanceState::Refused {
                    reason: reason.clone(),
                },
            );
            refuse(reason)
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
            return Err(refused(format!(
                "the command namespace `{id}` is already {owner}'s — give the instance another id"
            )));
        }
        if self.work_items.get(id).is_ok() {
            return Err(refused(format!(
                "`{id}` is already a provider — give the instance another id"
            )));
        }
        let instance = match self.instance(ext, spec, id, scope, config).await {
            Ok(i) => i,
            Err(e) => {
                self.start_failed(&name, None, e.clone()).await;
                return Err(e);
            }
        };
        let vocabulary = bus.vocabulary().current();
        for t in &instance.declared.event_types {
            if vocabulary.schema(&t.event_type, t.v) != Some(&t.schema) {
                return Err(refused(format!(
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
                self.admit(&bus, instance.clone(), epoch)
                    .await
                    .map_err(&refuse)?;
                self.failed(&name, Some(&instance), e.to_string()).await;
                Ok(())
            }
            Err(e) => {
                self.start_failed(&name, None, e.clone()).await;
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
        self.deps.capabilities.set_external(
            crate::capabilities::Implementation {
                capability: capability.clone(),
                id: instance.id.clone(),
                title: instance.name.clone(),
                extension: Some(instance.ext.name.clone()),
                source: crate::capabilities::Source::External,
                features,
                fields: serde_json::to_value(&instance.spec.fields).unwrap_or_default(),
                id_pattern: instance.spec.id_pattern.clone(),
            },
            true,
        );
        let config = crate::config_service::read_config(&self.deps.config);
        if let Err(e) = self.deps.capabilities.publish(&config, &self.deps.db).await {
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
                    bus.unregister_source(&format!("provider:{}", instance.name));
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
            bus.unregister_source(&format!("provider:{}", running.name));
        }
        self.work_items.unregister(&running.id);
        self.deps.capabilities.set_external(
            crate::capabilities::Implementation {
                capability: running.spec.capability.clone(),
                id: running.id.clone(),
                title: running.name.clone(),
                extension: Some(running.ext.name.clone()),
                source: crate::capabilities::Source::External,
                features: Value::Null,
                fields: serde_json::Value::Array(Vec::new()),
                id_pattern: None,
            },
            false,
        );
        let config = crate::config_service::read_config(&self.deps.config);
        if let Err(e) = self.deps.capabilities.publish(&config, &self.deps.db).await {
            tracing::warn!(instance = %running.name, error = %e, "withdrawing a provider's features failed");
        }
        running
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        running.live.lock().await.take();
    }

    /// A person approved program `<extension>/<provider id>` as it is
    /// now: every running instance of it restarts on what was approved
    /// (its declarations may have changed, and a start checks them against
    /// what it was enabled with), and an enabled one that was down for want
    /// of approval starts (tsk1062).
    pub async fn approved(&self, program: &str) {
        let of_it: Vec<String> = self
            .running
            .lock()
            .await
            .values()
            .filter(|i| i.spec.approval_name(&i.ext.name) == program)
            .map(|i| i.name.clone())
            .collect();
        for name in of_it {
            self.stop(&name).await;
        }
        self.reconcile().await;
    }

    /// A person's Check / Enable / Disable on Settings → Integrations:
    /// write `extensionInstances.<instance>` through `oxplow.config.set` and, to
    /// enable, run `oxplow.plugin.enable`. Enabling checks first: an
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
        {
            // The instances as they are now, not as they were before the
            // check: another row's change meanwhile stands.
            let gate = self.instances_gate.lock().await;
            let Resolved { scope, .. } =
                self.resolve(instance)
                    .map_err(|message| CommandError::Invalid {
                        field: Some("/instance".into()),
                        message,
                    })?;
            // It is written where it lives: the project's entry (its own,
            // or its replacement of a global one), else the machine's file.
            let home = if scope == Scope::Global && !self.project_instances().contains_key(instance)
            {
                Scope::Global
            } else {
                Scope::Project
            };
            self.write_instances(&gate, actor, home, |all| {
                let was = all.get(instance);
                let sync_minutes = was.and_then(|c| c.sync_minutes);
                let provider = was.and_then(|c| c.provider.clone());
                all.insert(
                    instance.to_string(),
                    oxplow_config::ExtensionInstanceConfig {
                        enabled,
                        config,
                        sync_minutes,
                        provider,
                    },
                );
                Ok(())
            })
            .await?;
        }
        self.reconcile().await;
        self.view(instance).await
    }

    /// A person adds another instance of `provider`: `instance`
    /// (`<extension>/<instance id>`), off and unconfigured until they set
    /// it up — the project's (written through `oxplow.config.set`:
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
        if id != provider && ext.providers.iter().any(|p| p.id == id) {
            return Err(invalid(format!(
                "`{id}` is already the id of one of `{}`'s providers (its default instance)",
                ext.name
            )));
        }
        {
            let gate = self.instances_gate.lock().await;
            if self.instances_config().contains_key(instance) {
                return Err(invalid(format!("`{instance}` is already an instance")));
            }
            self.write_instances(&gate, actor, scope, |all| {
                if all.contains_key(instance) {
                    return Err(invalid(format!("`{instance}` is already an instance")));
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
                Ok(())
            })
            .await?;
        }
        self.reconcile().await;
        self.view(instance).await
    }

    /// A person turns the global instance `instance` off in this project
    /// only (tsk843): the project gets its own entry of the same name —
    /// off, with the global one's config and provider — which replaces it
    /// here (and is the project's, credentials included, tsk838); every
    /// other project keeps running it, and removing the entry brings it
    /// back here. Written through `oxplow.config.set`, so an agent's is refused
    /// or proposed.
    pub async fn off_here(
        &self,
        actor: &Actor,
        instance: &str,
    ) -> Result<ProviderInstanceView, CommandError> {
        let invalid = |message: String| CommandError::Invalid {
            field: Some("/instance".into()),
            message,
        };
        {
            let gate = self.instances_gate.lock().await;
            if self.project_instances().contains_key(instance) {
                return Err(invalid(format!(
                    "`{instance}` already has this project's own entry"
                )));
            }
            let Some(global) = self.global_instances().get(instance).cloned() else {
                return Err(invalid(format!(
                    "`{instance}` isn't one of your global instances"
                )));
            };
            self.write_instances(&gate, actor, Scope::Project, |all| {
                all.insert(
                    instance.to_string(),
                    oxplow_config::ExtensionInstanceConfig {
                        enabled: false,
                        ..global
                    },
                );
                Ok(())
            })
            .await?;
        }
        self.reconcile().await;
        self.view(instance).await
    }

    /// A person removes `instance`: it stops, its config entry and its
    /// credentials on this machine go. A project's replacement of a global
    /// instance is what goes first — with its own credentials; the global
    /// one then shows through, on its own.
    pub async fn remove_instance(&self, actor: &Actor, instance: &str) -> Result<(), CommandError> {
        // A sign-in under way would store a token for what's gone — or,
        // for a project's replacement of a global one, under the removed
        // entry's account (tsk826).
        self.abandon_sign_ins(|(of, _)| of == instance).await;
        let missing = || CommandError::Invalid {
            field: Some("/instance".into()),
            message: format!("no provider instance `{instance}`"),
        };
        let (gone, home, still_there) = {
            let gate = self.instances_gate.lock().await;
            let resolved = self.resolve(instance).ok();
            let configured = self.instances_config().get(instance).cloned();
            let in_project = self.project_instances().contains_key(instance);
            let in_global = self.global_instances().contains_key(instance);
            // A project's replacement of a global one: the global one stays.
            let home = match (in_project, in_global) {
                (true, _) => Scope::Project,
                (false, true) => Scope::Global,
                (false, false) => return Err(missing()),
            };
            self.write_instances(&gate, actor, home, |all| {
                all.remove(instance).map(|_| ()).ok_or_else(missing)
            })
            .await?;
            (
                Gone::of(instance, resolved, configured.as_ref()),
                home,
                in_project && in_global,
            )
        };
        self.reconcile().await;
        // Its credentials go with it — a project's replacement's are its
        // own, never the global one's that now shows through (tsk838) —
        // named by the extension, or, when it no longer declares the
        // provider (when Remove is offered), by the copy it last ran
        // (tsk841).
        let names = match &gone.credentials {
            Some(names) => names.clone(),
            None => host::last_ran_credentials(&self.deps.copies, &gone.ext, &gone.provider),
        };
        for name in &names {
            let account = crate::collector_runner::instance_credential_account(
                credential_scope(&self.deps, home),
                &gone.ext,
                &gone.id,
                name,
            );
            if let Err(e) = self.deps.secrets.delete(&account) {
                tracing::warn!(%instance, credential = %name, error = %e, "removing an instance's credential failed");
            }
        }
        // The global one showing through keeps what it has.
        if still_there {
            return Ok(());
        }
        self.health.lock().remove(instance);
        if let Err(e) = self.plugins.forget(&plugin_key(instance)).await {
            tracing::warn!(%instance, error = %e, "forgetting an instance's health failed");
        }
        if let Err(e) = oxplow_db::SqliteProviderCollectorStore::new(self.deps.db.clone())
            .remove_instance(instance)
            .await
        {
            tracing::warn!(%instance, error = %e, "forgetting an instance's reads failed");
        }
        // No capability's active provider is one that's gone.
        let config = crate::config_service::read_config(&self.deps.config);
        let mut active = config.active_providers.clone();
        active.retain(|_, id| *id != gone.id);
        if active != config.active_providers {
            let bus = self.bus.upgrade().ok_or_else(|| CommandError::Failed {
                message: "the command bus is gone".into(),
            })?;
            bus.run(
                actor,
                crate::commands::config_commands::SET,
                json!({ "key": "activeProviders", "value": active }),
                matches!(actor, Actor::Human),
            )
            .await?;
        }
        Ok(())
    }

    /// Change `scope`'s instances as `actor` by `edit`, which sees them as
    /// they are now — under the `instances_gate` the caller holds (`_gate`)
    /// and, for the machine's file, its cross-process lock: the project's
    /// through `oxplow.config.set` (`extensionInstances` is a person's key), the
    /// machine's file directly — a person's only: no command reaches it,
    /// so no agent or lens can. The caller reconciles once it lets go of
    /// the gate.
    async fn write_instances(
        &self,
        _gate: &tokio::sync::MutexGuard<'_, ()>,
        actor: &Actor,
        scope: Scope,
        edit: impl FnOnce(
            &mut BTreeMap<String, oxplow_config::ExtensionInstanceConfig>,
        ) -> Result<(), CommandError>,
    ) -> Result<(), CommandError> {
        match scope {
            Scope::Project => {
                let bus = self.bus.upgrade().ok_or_else(|| CommandError::Failed {
                    message: "the command bus is gone".into(),
                })?;
                let mut all = self.project_instances();
                edit(&mut all)?;
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
                oxplow_config::GlobalInstances::update(&dir, edit).map_err(|e| {
                    CommandError::Failed {
                        message: e.to_string(),
                    }
                })??;
            }
        }
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
    /// running one), or starts if the credential was what it lacked. A
    /// global instance's runs in every project: the change is counted
    /// where every other oxplow on the machine looks, and each restarts
    /// it on its next tick (tsk842).
    pub async fn credential_changed(&self, instance: &str) {
        if let (Scope::Global, Some(dir)) = (self.scope_of(instance).0, &self.deps.global_dir) {
            match oxplow_config::CredentialGenerations::bump(dir, instance) {
                Ok(n) => {
                    self.global
                        .lock()
                        .credentials_seen
                        .insert(instance.to_string(), n);
                }
                Err(e) => {
                    tracing::warn!(%instance, error = %e, "telling other projects its credentials changed failed")
                }
            }
        }
        self.stop(instance).await;
        self.reconcile().await;
    }

    /// Start signing in for `instance`'s credential `name` (one declared
    /// with `oauth:`), its redirect coming back to `redirect_port` on the
    /// person's machine, where the shell listens: where the person goes
    /// to do it. The shell hands the redirect back by
    /// [`Self::complete_sign_in`]. A sign-in already under way for it is
    /// abandoned; one never finished ends after [`oauth::SIGN_IN_WAIT`],
    /// and the renderer hears `CredentialChanged` saying so.
    pub async fn begin_sign_in(
        &self,
        instance: &str,
        name: &str,
        redirect_port: u16,
    ) -> Result<BegunSignIn, DomainError> {
        let (account, decl, resolved) = self.credential(instance, name)?;
        let Some(oauth_decl) = decl.oauth else {
            return Err(DomainError::Invalid(format!(
                "`{name}` isn't a credential you sign in for; paste its value instead"
            )));
        };
        // Where it signs in, and where the code, the verifier and the
        // client secret go, are part of what a person approved: only as
        // they were approved (tsk824). The declaration kept below is the
        // one this checks, and finishing checks it again.
        self.approved_for_sign_in(&resolved, name)?;
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
        let sign_in = oauth::begin(&oauth_decl, client_secret, redirect_port)
            .map_err(|e| DomainError::Invalid(format!("signing in for `{name}`: {e}")))?;
        let key = (instance.to_string(), name.to_string());
        // The old one is gone before the new one is tracked, and no finish
        // is half done meanwhile.
        let gate = self.sign_in_gate(&key);
        let _gate = gate.lock().await;
        // The one it replaces is told — another window's row may be
        // waiting on it (tsk929).
        for replaced in self.abandon_sign_ins_locked(|k| *k == key) {
            self.sign_in_news(&key, replaced, Some("a newer sign-in replaced it".into()));
        }
        let seq = self
            .sign_in_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let expiry = tokio::spawn({
            let (me, key) = (self.me.clone(), key.clone());
            async move {
                tokio::time::sleep(oauth::SIGN_IN_WAIT).await;
                if let Some(registry) = me.upgrade() {
                    registry.expire_sign_in(key, seq).await;
                }
            }
        });
        self.sign_ins.lock().insert(
            key,
            SignInUnderWay {
                seq,
                account,
                pending: sign_in.pending,
                expiry,
            },
        );
        Ok(BegunSignIn {
            url: sign_in.authorize_url,
            sign_in: seq,
        })
    }

    /// A person ends sign-in `sign_in` for `instance`'s `name` (the row was
    /// left, or its browser never opened): nothing of it is kept — its
    /// verifier and client secret go now, not in five minutes — and its
    /// news says so (tsk929). One already over is nothing to cancel.
    pub async fn cancel_sign_in(&self, instance: &str, name: &str, sign_in: SignInId) {
        let key = (instance.to_string(), name.to_string());
        let gate = self.sign_in_gate(&key);
        let _gate = gate.lock().await;
        let cancelled = {
            let mut sign_ins = self.sign_ins.lock();
            if sign_ins.get(&key).is_some_and(|s| s.seq == sign_in) {
                sign_ins.remove(&key)
            } else {
                None
            }
        };
        if let Some(cancelled) = cancelled {
            cancelled.expiry.abort();
            self.sign_in_news(&key, sign_in, Some("the sign-in was cancelled".into()));
        }
    }

    /// Tell the renderer how sign-in `sign_in` for `key` went
    /// (`CredentialChanged`, naming it).
    fn sign_in_news(&self, key: &(String, String), sign_in: SignInId, error: Option<String>) {
        self.deps
            .events
            .emit(crate::events::OxplowEvent::CredentialChanged {
                instance: key.0.clone(),
                name: key.1.clone(),
                sign_in: Some(sign_in),
                error,
            });
    }

    /// Refused unless `resolved`'s provider is approved as it is now.
    fn approved_for_sign_in(&self, resolved: &Resolved, name: &str) -> Result<(), DomainError> {
        if crate::exec_consent::may_run_provider(
            &self.deps.approvals,
            &self.deps.project_dir,
            &resolved.ext,
            &resolved.spec,
        ) {
            return Ok(());
        }
        Err(DomainError::Invalid(format!(
            "provider `{}` isn't approved as it is now (where `{name}` signs in is part of \
             what you approve): approve it on Settings → Data → Programs first",
            resolved.spec.approval_name(&resolved.ext.name)
        )))
    }

    /// The redirect the shell caught for `instance`'s sign-in for `name`:
    /// `redirect` is the path and query the browser asked for. One that
    /// isn't that sign-in's is refused and the sign-in waits on; one that
    /// is ends it — the code exchanged (the provider re-checked as
    /// approved, with the declaration it began with) and the token kept —
    /// and answers then, so the browser hears at once; the instance
    /// restarts on it afterwards and the renderer is told
    /// (`CredentialChanged`) when it has (tsk906). Once the sign-in is
    /// taken off the waiting list the finish runs to its end even if the
    /// caller goes away (a dropped connection to a remote daemon): it is
    /// never left half done. With no sign-in under way (never begun,
    /// finished, expired, abandoned) it is an error.
    pub async fn complete_sign_in(
        &self,
        instance: &str,
        name: &str,
        redirect: &str,
    ) -> Result<SignInCompletion, DomainError> {
        let me = self
            .me
            .upgrade()
            .ok_or_else(|| DomainError::Invalid("oxplow is shutting down".into()))?;
        let (instance, name, redirect) =
            (instance.to_string(), name.to_string(), redirect.to_string());
        tokio::spawn(async move { me.finish_sign_in(instance, name, redirect).await })
            .await
            .map_err(|e| DomainError::Invariant(format!("finishing the sign-in: {e}")))?
    }

    /// [`Self::complete_sign_in`]'s work, on a task of its own.
    async fn finish_sign_in(
        self: Arc<Self>,
        instance: String,
        name: String,
        redirect: String,
    ) -> Result<SignInCompletion, DomainError> {
        let key = (instance.clone(), name.clone());
        let gate = self.sign_in_gate(&key);
        let gate = gate.lock().await;
        let redirected = {
            let sign_ins = self.sign_ins.lock();
            let Some(under_way) = sign_ins.get(&key) else {
                return Err(DomainError::Invalid(format!(
                    "no sign-in for `{name}` of `{instance}` is under way"
                )));
            };
            match under_way.pending.redirected(&redirect) {
                Ok(redirected) => redirected,
                Err(reason) => return Ok(SignInCompletion::NotThisSignIn { reason }),
            }
        };
        let Some(under_way) = self.sign_ins.lock().remove(&key) else {
            return Err(DomainError::Invalid(format!(
                "no sign-in for `{name}` of `{instance}` is under way"
            )));
        };
        under_way.expiry.abort();
        let outcome = match redirected {
            oauth::Redirected::Refused(why) => Err(why),
            oauth::Redirected::Code(code) => {
                match self.still_approved(&instance, &name, &under_way) {
                    Err(why) => Err(why),
                    Ok(()) => match under_way.pending.exchange(&code).await {
                        Ok(token) => {
                            oauth::store(self.deps.secrets.as_ref(), &under_way.account, &token)
                        }
                        Err(why) => Err(why),
                    },
                }
            }
        };
        drop(gate);
        // The restart and the renderer's news follow on their own: the
        // browser isn't kept waiting for a slow `check`.
        let (me, error, seq) = (self.clone(), outcome.clone().err(), under_way.seq);
        tokio::spawn(async move {
            if error.is_none() {
                me.credential_changed(&instance).await;
            }
            me.sign_in_news(&(instance, name), seq, error);
        });
        Ok(match outcome {
            Ok(()) => SignInCompletion::SignedIn,
            Err(error) => SignInCompletion::Failed { error },
        })
    }

    /// A sign-in being finished may send its code, verifier and client
    /// secret only where it was approved to: its provider is approved as
    /// it is now, still declares the sign-in it began with, and the
    /// instance still keeps its credentials where it did — a global one
    /// turned off here, or a project entry of its name arriving, moves
    /// them, and the token mustn't go to the old place (tsk909).
    fn still_approved(
        &self,
        instance: &str,
        name: &str,
        under_way: &SignInUnderWay,
    ) -> Result<(), String> {
        let (account, decl, resolved) =
            self.credential(instance, name).map_err(|e| e.to_string())?;
        if account != under_way.account {
            return Err(format!(
                "`{instance}` keeps its credentials elsewhere since the sign-in began; sign in again"
            ));
        }
        self.approved_for_sign_in(&resolved, name)
            .map_err(|e| e.to_string())?;
        if decl.oauth.as_ref() != Some(under_way.pending.decl()) {
            return Err(format!(
                "where `{name}` signs in changed since the sign-in began; sign in again"
            ));
        }
        Ok(())
    }

    /// Sign-in `seq` for `key` was never finished: end it, and tell the
    /// renderer.
    async fn expire_sign_in(&self, key: (String, String), seq: SignInId) {
        let gate = self.sign_in_gate(&key);
        let _gate = gate.lock().await;
        let expired = {
            let mut sign_ins = self.sign_ins.lock();
            if sign_ins.get(&key).is_some_and(|s| s.seq == seq) {
                sign_ins.remove(&key)
            } else {
                None
            }
        };
        if expired.is_some() {
            self.sign_in_news(
                &key,
                seq,
                Some(format!(
                    "the sign-in wasn't finished within {} minutes",
                    oauth::SIGN_IN_WAIT.as_secs() / 60
                )),
            );
        }
    }

    /// End the sign-ins `which` picks, keeping nothing; one being finished
    /// is waited for first (each picked credential's gate, in key order).
    async fn abandon_sign_ins(&self, which: impl Fn(&(String, String)) -> bool) {
        let gates: Vec<_> = self
            .sign_in_gates
            .lock()
            .iter()
            .filter(|(k, _)| which(k))
            .map(|(_, g)| g.clone())
            .collect();
        let mut held = Vec::with_capacity(gates.len());
        for gate in &gates {
            held.push(gate.lock().await);
        }
        self.abandon_sign_ins_locked(which);
    }

    /// The gate of the sign-in for `key` (`(instance, credential)`).
    fn sign_in_gate(&self, key: &(String, String)) -> SignInGate {
        self.sign_in_gates
            .lock()
            .entry(key.clone())
            .or_default()
            .clone()
    }

    /// [`Self::abandon_sign_ins`], the gates already held.
    /// The numbers of the sign-ins it ended.
    fn abandon_sign_ins_locked(&self, which: impl Fn(&(String, String)) -> bool) -> Vec<SignInId> {
        let mut sign_ins = self.sign_ins.lock();
        let keys: Vec<_> = sign_ins.keys().filter(|k| which(k)).cloned().collect();
        let mut ended = Vec::new();
        for key in keys {
            if let Some(abandoned) = sign_ins.remove(&key) {
                abandoned.expiry.abort();
                ended.push(abandoned.seq);
            }
        }
        ended
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

    /// A person enabled `instance` again (`oxplow.plugin.enable`): its failure
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
    /// `made_by` is the running instance whose (re)start it was; a start
    /// of one no longer running is no one's, and changes nothing — never
    /// the state, health or enablement of the instance now running under
    /// its name (tsk836). `None`: an enable's first start, before the
    /// instance runs.
    async fn start_failed(&self, instance: &str, made_by: Option<&Instance>, e: HostError) {
        let unstarted = match e {
            HostError::Unapproved(_) => InstanceState::Unapproved,
            HostError::Unconfigured { problems, .. } => InstanceState::Unconfigured {
                problems: problems
                    .into_iter()
                    .map(|p| ConfigProblem {
                        path: p.path,
                        message: p.message,
                    })
                    .collect(),
            },
            // It isn't what was approved: off until a person looks.
            HostError::DeclarationsChanged { .. } => {
                return self.disable(instance, made_by, e.to_string()).await;
            }
            HostError::Failed { .. } => return self.failed(instance, made_by, e.to_string()).await,
        };
        if let Some(made_by) = made_by {
            let Some(running) = self.take_if_current(made_by).await else {
                return;
            };
            self.tear_down(running).await;
        }
        self.set_state(instance, unstarted);
    }

    /// Remove `made_by` from the running instances if it is the one running
    /// under its name.
    async fn take_if_current(&self, made_by: &Instance) -> Option<Arc<Instance>> {
        let mut running = self.running.lock().await;
        if !is_running(&running, made_by) {
            return None;
        }
        running.remove(&made_by.name)
    }

    /// A start or call failed: count it ([`crate::plugin_health`]); the
    /// verdict either backs the next start off or stops the instance.
    /// `made_by` as for [`Self::start_failed`]: the count, the health and
    /// any stop are its while it runs — checked and written under the
    /// lock a stop takes, so a successor never inherits them.
    pub(super) async fn failed(&self, instance: &str, made_by: Option<&Instance>, error: String) {
        let mut running = self.running.lock().await;
        if made_by.is_some_and(|i| !is_running(&running, i)) {
            return;
        }
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
            crate::plugin_health::Verdict::Disabled { reason } => {
                let removed = self.halt_locked(&mut running, instance);
                drop(running);
                self.halted(instance, removed, reason).await;
            }
            crate::plugin_health::Verdict::Backoff { .. } => {
                if let Some(i) = running.get(instance) {
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
    /// enables it, for `reason` (`plugin.disabled@1`) — when `made_by` is
    /// given, only while it is the one running.
    pub(super) async fn disable(&self, instance: &str, made_by: Option<&Instance>, reason: String) {
        let mut running = self.running.lock().await;
        if made_by.is_some_and(|i| !is_running(&running, i)) {
            return;
        }
        if let Err(e) = self.plugins.disable(&plugin_key(instance), &reason).await {
            tracing::error!(instance, error = %e, "recording the disable failed");
        }
        let removed = self.halt_locked(&mut running, instance);
        drop(running);
        self.halted(instance, removed, reason).await;
    }

    /// Take a disabled `instance` out of the running ones, under their
    /// lock: a start under way (its epoch) won't admit it either.
    fn halt_locked(
        &self,
        running: &mut BTreeMap<String, Arc<Instance>>,
        instance: &str,
    ) -> Option<Arc<Instance>> {
        *self
            .disables
            .lock()
            .entry(instance.to_string())
            .or_default() += 1;
        running.remove(instance)
    }

    /// End what [`Self::halt_locked`] took, and show why.
    async fn halted(&self, instance: &str, removed: Option<Arc<Instance>>, reason: String) {
        if let Some(running) = removed {
            self.tear_down(running).await;
        }
        self.set_state(instance, InstanceState::Disabled { reason });
    }

    /// Its service said to wait (`wait`, when it said how long) — while
    /// `made_by` is the running instance.
    pub(super) async fn note_rate_limit(&self, made_by: &Instance, wait: Option<Duration>) {
        let until = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms()
                + wait.unwrap_or(RATE_LIMIT_WAIT_MAX).as_millis() as i64,
        );
        let running = self.running.lock().await;
        if !is_running(&running, made_by) {
            return;
        }
        if let Some(h) = self.health.lock().get_mut(&made_by.name) {
            h.rate_limited_until = Some(until.to_string());
        }
    }

    /// What a read in progress says it's doing; `None` when it ends —
    /// while `made_by` is the running instance.
    pub(super) async fn set_activity(&self, made_by: &Instance, activity: Option<String>) {
        let running = self.running.lock().await;
        if !is_running(&running, made_by) {
            return;
        }
        if let Some(h) = self.health.lock().get_mut(&made_by.name) {
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

    /// A call `instance` made failed: counted like any failure, while it
    /// is the running instance (tsk820). A call cut short by its own stop
    /// isn't one.
    pub(super) async fn call_failed(&self, instance: &Instance, error: String) {
        self.failed(&instance.name, Some(instance), error).await;
    }

    /// A call `instance` made succeeded: its health says so, while it is
    /// the running instance — checked and written under the lock a stop
    /// takes, so a stop either sees this and replaces it, or has already
    /// removed the instance.
    pub(super) async fn call_succeeded(&self, made_by: &Instance, took: Duration) {
        let running = self.running.lock().await;
        if !is_running(&running, made_by) {
            return;
        }
        let instance = made_by.name.as_str();
        if let Err(e) = self
            .plugins
            .succeeded(&plugin_key(instance), Some(took))
            .await
        {
            tracing::warn!(instance, error = %e, "recording its health failed");
        }
        let ms = took.as_secs_f64() * 1000.0;
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

/// Whether `instance` is the one running under its name: a stopped
/// instance's late results — and those of the one a restart replaced —
/// aren't the running one's (tsk820, tsk836).
fn is_running(running: &BTreeMap<String, Arc<Instance>>, instance: &Instance) -> bool {
    running
        .get(&instance.name)
        .is_some_and(|r| std::ptr::eq(Arc::as_ptr(r), instance))
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
                id: super::command_id(&id, &instance.spec.capability, &decl.name),
                summary: format!("{} (provider `{}`)", decl.summary, instance.name),
                input_schema: decl.input_schema.clone(),
                invokers: Invokers::ALL,
                confirm: host::confirm_of(&decl.confirm)?,
                undoable: decl.undoable,
                lifecycle: Lifecycle::Experimental,
                atomicity: Atomicity::External,
                effect: host::effect_of(&decl.effect)?,
                needs: Vec::new(),
                ui: None,
            };
            let (instance, verb, id) = (instance.clone(), decl.name.clone(), id.clone());
            Command::new(
                spec,
                Handler::External(Arc::new(
                    move |Invocation {
                              actor,
                              idempotency_key,
                          },
                          input| {
                        let (instance, verb, id) = (instance.clone(), verb.clone(), id.clone());
                        let capability = instance.spec.capability.clone();
                        Box::pin(async move {
                            let out = instance.invoke(&verb, input, idempotency_key).await?;
                            let events = out
                                .events
                                .into_iter()
                                .map(|d| instance.envelope(&actor, None, d))
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok(HandlerOutput {
                                result: out.result,
                                inverse: out.inverse.map(|c| CommandCall {
                                    name: super::command_id(&id, &capability, &c.command),
                                    input: c.input,
                                }),
                                events,
                                after_commit: None,
                                unchanged: false,
                            })
                        })
                    },
                )),
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
