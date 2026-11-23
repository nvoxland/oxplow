//! Event payload schemas: one Rust type per `type@v`, a registry that
//! validates payloads on append, and the golden-schema discipline that
//! keeps published event shapes from drifting.
//!
//! **Golden schemas.** Every registered core type's JSON Schema is
//! checked in at `crates/oxplow-domain/schemas/events/<type>@<v>.json`.
//! A test regenerates each schema from its Rust type and fails if the
//! file differs: a published `type@v` is a contract consumers (extensions,
//! pages, the agent) were written against, so a change is a **new
//! version** (`V + 1`, with an `upcast` from the version before), never an
//! edit. Run the test with `OXPLOW_BLESS=1` to write a new golden.
//!
//! **Namespaces.** Core types live in the namespaces of
//! [`CORE_NAMESPACES`] (`.context/data-model.md` "event_log"). An
//! extension declares types (a JSON Schema each, not a Rust type —
//! [`DeclaredEventType`]) only under its manifest's `namespace:`
//! (default [`extension_namespace`] of its name); declaring into a core
//! namespace, or under another, is refused. Extensions sharing a namespace
//! (everything oxplow ships is `oxplow`) share its names: each type is
//! still one registrant's.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{validate_type_name, Envelope};
use crate::DomainError;

/// The namespaces core owns (§5.3). Extension types may not use them.
pub const CORE_NAMESPACES: &[&str] = &[
    "agent",
    "capability",
    "thread",
    "stream",
    "snapshot",
    "vcs",
    "knowledge",
    "record",
    "test",
    "code",
    "command",
    "effect",
    "collector",
    "config",
    "provider",
    "contribution",
    "ui",
    "file",
    "terminal",
    "change",
    // No events: the request and response bodies of oxplow's own model
    // calls (`ai_call`) are kept under it in `event_content`.
    "ai",
];

/// One event type at one schema version. `Payload` is the Rust shape
/// the schema is generated from; `upcast` carries an older version's
/// payload forward so a consumer only ever reads the newest shape.
pub trait EventType {
    const TYPE: &'static str;
    const V: u32;
    type Payload: Serialize + DeserializeOwned + JsonSchema;

    /// Rewrite a payload written at `from_v` (`< V`) into this version's
    /// shape. The default knows no older versions; a type that bumps `V`
    /// overrides this with the chain `from_v → from_v + 1 → … → V`.
    fn upcast(from_v: u32, _payload: Value) -> Result<Value, DomainError> {
        Err(DomainError::Invalid(format!(
            "{}@{from_v} cannot be upcast to v{}: no upcast defined",
            Self::TYPE,
            Self::V
        )))
    }
}

/// The JSON Schema (draft 2020-12, as schemars emits it) for `T`.
pub fn schema_for<T: EventType>() -> Value {
    serde_json::to_value(schemars::schema_for!(T::Payload)).expect("schema serializes")
}

/// Carries a payload written at an older version (the `u32`) to the
/// version that declares it.
pub type Upcast = Arc<dyn Fn(u32, Value) -> Result<Value, DomainError> + Send + Sync>;

/// An extension's event type at one version: what its manifest's
/// `event_types:` declares, with the upcast its Starlark compiles to.
#[derive(Clone)]
pub struct DeclaredEventType {
    pub event_type: String,
    pub v: u32,
    /// The payload's JSON Schema.
    pub schema: Value,
    pub summary: String,
    /// Required past v1: carries every older version to this one.
    pub upcast: Option<Upcast>,
}

pub use crate::contribution::extension_namespace;

#[derive(Clone)]
struct Registered {
    /// Shared, so a registry clones cheaply (a review's copy plus one
    /// version's declarations).
    validator: Arc<jsonschema::Validator>,
    schema: Value,
    /// Who registered it: `None` for core, `Some(extension)` otherwise.
    owner: Option<String>,
    /// What it records; core types say so in their Rust docs instead.
    summary: Option<String>,
}

/// Every event type the log accepts, by `type@v`, with the newest version
/// of each type and its upcast chain.
#[derive(Default, Clone)]
pub struct EventSchemaRegistry {
    by_version: HashMap<(String, u32), Registered>,
    latest: HashMap<String, u32>,
    upcasts: HashMap<String, Upcast>,
}

impl EventSchemaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry with every core type registered.
    pub fn core() -> Self {
        let mut r = Self::new();
        r.register::<CommandExecuted>()
            .expect("core type registers");
        r.register::<CapabilitySwitched>()
            .expect("core type registers");
        r.register::<ConfigChanged>().expect("core type registers");
        r.register::<FileSaved>().expect("core type registers");
        r.register::<FileCreated>().expect("core type registers");
        r.register::<FileDirectoryCreated>()
            .expect("core type registers");
        r.register::<FileRenamed>().expect("core type registers");
        r.register::<FileDeleted>().expect("core type registers");
        r.register::<TerminalOpened>().expect("core type registers");
        r.register::<TerminalRenamed>()
            .expect("core type registers");
        r.register::<TerminalClosed>().expect("core type registers");
        r.register::<EffectResult>().expect("core type registers");
        r.register::<SnapshotTaken>().expect("core type registers");
        r.register::<VcsHeadMoved>().expect("core type registers");
        r.register::<VcsCommitIndexed>()
            .expect("core type registers");
        r.register::<AgentTurnStarted>()
            .expect("core type registers");
        r.register::<AgentTurnEnded>().expect("core type registers");
        r.register::<AgentSessionStarted>()
            .expect("core type registers");
        r.register::<AgentSessionEnded>()
            .expect("core type registers");
        r.register::<AgentPromptSubmitted>()
            .expect("core type registers");
        r.register::<AgentToolRequested>()
            .expect("core type registers");
        r.register::<AgentToolFinished>()
            .expect("core type registers");
        r.register::<AgentSubagentStarted>()
            .expect("core type registers");
        r.register::<AgentSubagentFinished>()
            .expect("core type registers");
        r.register::<AgentContextCompacted>()
            .expect("core type registers");
        r.register::<AgentStatusChanged>()
            .expect("core type registers");
        r.register::<AgentTokensReported>()
            .expect("core type registers");
        r.register::<TestRunRecorded>()
            .expect("core type registers");
        r.register::<TestCoverageRecorded>()
            .expect("core type registers");
        r.register::<CollectorSynced>()
            .expect("core type registers");
        r.register::<ThreadCheckpoint>()
            .expect("core type registers");
        r.register::<RecordRead>().expect("core type registers");
        r.register::<RefDrifted>().expect("core type registers");
        r.register::<ThreadClosed>().expect("core type registers");
        r.register::<StreamArchived>().expect("core type registers");
        r.register::<ChangeAnalyzed>().expect("core type registers");
        r.register::<CodeDiagnosticsChanged>()
            .expect("core type registers");
        r.register::<ContributionEnabled>()
            .expect("core type registers");
        r.register::<ContributionDisabled>()
            .expect("core type registers");
        r.register::<ContributionJobCompleted>()
            .expect("core type registers");
        r.register::<CommandProposed>()
            .expect("core type registers");
        r.register::<CommandApproved>()
            .expect("core type registers");
        r.register::<CommandDeclined>()
            .expect("core type registers");
        r.register::<UiOpFailed>().expect("core type registers");
        r.register::<UiOpErrorSeen>().expect("core type registers");
        r.register::<UiOpErrorDismissed>()
            .expect("core type registers");
        r
    }

    /// Register a core type. Its namespace must be a core namespace.
    pub fn register<T: EventType>(&mut self) -> Result<(), DomainError> {
        let ns = namespace_of(T::TYPE);
        if !CORE_NAMESPACES.contains(&ns) {
            return Err(DomainError::Invalid(format!(
                "`{}` is not in a core namespace; an extension declares it with `register_declared`",
                T::TYPE
            )));
        }
        let upcast: Upcast = Arc::new(|from, payload| T::upcast(from, payload));
        self.insert(T::TYPE, T::V, schema_for::<T>(), None, None, Some(upcast))
    }

    /// Register an extension's declared type. It must be under the
    /// extension's `namespace` (its manifest's `namespace:`, not its name:
    /// extensions sharing a namespace share its names) — never a core
    /// namespace, never another's; its schema must compile; past v1 it must
    /// carry an upcast. The registrant is `extension`, which alone may emit
    /// it.
    pub fn register_declared(
        &mut self,
        extension: &str,
        namespace: &str,
        declared: DeclaredEventType,
    ) -> Result<(), DomainError> {
        let DeclaredEventType {
            event_type,
            v,
            schema,
            summary,
            upcast,
        } = declared;
        validate_type_name(&event_type)?;
        let ns = namespace_of(&event_type);
        if CORE_NAMESPACES.contains(&ns) {
            return Err(DomainError::Invalid(format!(
                "extension `{extension}` may not declare `{event_type}`: `{ns}` is a core namespace"
            )));
        }
        if ns != namespace {
            return Err(DomainError::Invalid(format!(
                "extension `{extension}` may only declare types under `{namespace}.*`, not `{event_type}`"
            )));
        }
        if v == 0 {
            return Err(DomainError::Invalid(format!(
                "`{event_type}@{v}`: versions start at 1, not v0"
            )));
        }
        if v > 1 && upcast.is_none() {
            return Err(DomainError::Invalid(format!(
                "`{event_type}@{v}` needs an `upcast` carrying older versions to v{v}"
            )));
        }
        self.insert(
            &event_type,
            v,
            schema,
            Some(extension.to_string()),
            Some(summary),
            upcast,
        )
    }

    fn insert(
        &mut self,
        event_type: &str,
        v: u32,
        schema: Value,
        owner: Option<String>,
        summary: Option<String>,
        upcast: Option<Upcast>,
    ) -> Result<(), DomainError> {
        validate_type_name(event_type)?;
        let key = (event_type.to_string(), v);
        if self.by_version.contains_key(&key) {
            return Err(DomainError::Invalid(format!(
                "event type `{event_type}@{v}` is already registered"
            )));
        }
        let validator = jsonschema::validator_for(&schema).map_err(|e| {
            DomainError::Invalid(format!(
                "the schema for `{event_type}@{v}` doesn't compile: {e}"
            ))
        })?;
        self.by_version.insert(
            key,
            Registered {
                validator: Arc::new(validator),
                schema,
                owner,
                summary,
            },
        );
        let latest = self.latest.entry(event_type.to_string()).or_insert(0);
        if v >= *latest {
            *latest = v;
            match upcast {
                Some(up) => self.upcasts.insert(event_type.to_string(), up),
                None => self.upcasts.remove(event_type),
            };
        }
        Ok(())
    }

    pub fn is_registered(&self, event_type: &str, v: u32) -> bool {
        self.by_version.contains_key(&(event_type.to_string(), v))
    }

    /// The newest registered version of `event_type`.
    pub fn latest(&self, event_type: &str) -> Option<u32> {
        self.latest.get(event_type).copied()
    }

    pub fn schema(&self, event_type: &str, v: u32) -> Option<&Value> {
        self.by_version
            .get(&(event_type.to_string(), v))
            .map(|r| &r.schema)
    }

    /// What a declared `type@v` records; `None` for core or unregistered.
    pub fn summary(&self, event_type: &str, v: u32) -> Option<&str> {
        self.by_version
            .get(&(event_type.to_string(), v))
            .and_then(|r| r.summary.as_deref())
    }

    /// Who registered `type@v`: `Some(None)` for core, `Some(Some(extension))`
    /// for an extension's, `None` when unregistered.
    pub fn owner(&self, event_type: &str, v: u32) -> Option<Option<&str>> {
        self.by_version
            .get(&(event_type.to_string(), v))
            .map(|r| r.owner.as_deref())
    }

    /// Every registered `(type, v)`, sorted.
    pub fn versions(&self) -> Vec<(String, u32)> {
        let mut out: Vec<_> = self.by_version.keys().cloned().collect();
        out.sort();
        out
    }

    /// Refuse a payload that is not valid for a registered `type@v`. The
    /// error names the type, the version and the first schema violation
    /// with its JSON path.
    pub fn validate(&self, event_type: &str, v: u32, payload: &Value) -> Result<(), DomainError> {
        let Some(reg) = self.by_version.get(&(event_type.to_string(), v)) else {
            return Err(DomainError::Invalid(match self.latest(event_type) {
                Some(latest) => {
                    format!("event type `{event_type}@{v}` is not registered (newest is v{latest})")
                }
                None => format!("event type `{event_type}` is not registered"),
            }));
        };
        match reg.validator.iter_errors(payload).next() {
            None => Ok(()),
            Some(err) => Err(DomainError::Invalid(format!(
                "`{event_type}@{v}` payload invalid at `{}`: {err}",
                err.instance_path()
            ))),
        }
    }

    /// Carry a payload written at `from_v` to the newest version of its
    /// type and validate it there. A payload already at the newest version
    /// is validated and returned unchanged.
    pub fn upcast_to_latest(
        &self,
        event_type: &str,
        from_v: u32,
        payload: Value,
    ) -> Result<(u32, Value), DomainError> {
        let latest = self.latest(event_type).ok_or_else(|| {
            DomainError::Invalid(format!("event type `{event_type}` is not registered"))
        })?;
        let value = if from_v == latest {
            payload
        } else if from_v < latest {
            let up = self.upcasts.get(event_type).ok_or_else(|| {
                DomainError::Invalid(format!(
                    "`{event_type}@{from_v}` has no upcast to the registered v{latest}"
                ))
            })?;
            up(from_v, payload)?
        } else {
            return Err(DomainError::Invalid(format!(
                "`{event_type}@{from_v}` is newer than the registered v{latest}"
            )));
        };
        self.validate(event_type, latest, &value)?;
        Ok((latest, value))
    }
}

/// The namespace of `event_type` when it could be an extension's: a
/// well-formed type name outside core's namespaces. An extension that
/// reacts to such a type, not its own, names its owner by it.
pub fn extension_type_namespace(event_type: &str) -> Option<&str> {
    validate_type_name(event_type).ok()?;
    let ns = namespace_of(event_type);
    (!CORE_NAMESPACES.contains(&ns)).then_some(ns)
}

fn namespace_of(event_type: &str) -> &str {
    event_type.split('.').next().unwrap_or("")
}

impl Envelope {
    /// A typed envelope: `type`/`v` from `T`, the payload serialized from
    /// `T::Payload`, so a core producer can't emit a shape its schema
    /// doesn't describe.
    pub fn typed<T: EventType>(source: impl Into<String>, payload: &T::Payload) -> Self {
        let value = serde_json::to_value(payload).expect("payload serializes");
        Self::new(T::TYPE, T::V, source, value).expect("core type names are valid")
    }
}

// ---------------------------------------------------------------------------
// Core types, v1
// ---------------------------------------------------------------------------

/// A job run ended, however it ended. What it was asked and answered stay
/// private job data: a consumer reads them through its extension's
/// `jobs.read` (`scope("jobs.read", { job_run })`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContributionJobCompletedV2 {
    /// The run's id (a UUID).
    pub job_run: String,
    /// The job it ran (`<extension>/<id>`).
    pub job: String,
    pub version: Option<u32>,
    /// Its last attempt (0 when it was cancelled before one started).
    pub attempt: u32,
    pub fence: u64,
    pub outcome: JobOutcome,
}

/// How a job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    Succeeded,
    Failed,
    /// Someone asked it to stop, and it did.
    Cancelled,
    /// It stopped without a known end: oxplow restarted, or it lost its
    /// lease.
    Interrupted,
}

pub struct ContributionJobCompleted;
impl EventType for ContributionJobCompleted {
    const TYPE: &'static str = "contribution.job.completed";
    const V: u32 = 2;
    type Payload = ContributionJobCompletedV2;
}

/// Who ran a command (`.context/target-architecture.md` §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    /// A page acting for whoever pressed it.
    Page,
    /// An extension's custom component, for whoever uses it.
    Component,
    System,
    /// An extension's effect (P8.D8).
    Effect,
    /// A durable host-managed extension job.
    Job,
}

/// Who ran a command (`.context/target-architecture.md` §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "ActorKind")]
pub enum ActorKindV2 {
    Human,
    Agent,
    Page,
    System,
    /// An extension's effect (P8.D8).
    Effect,
}

impl TryFrom<ActorKind> for ActorKindV2 {
    type Error = DomainError;
    fn try_from(kind: ActorKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            ActorKind::Human => Self::Human,
            ActorKind::Agent => Self::Agent,
            ActorKind::Page => Self::Page,
            ActorKind::System => Self::System,
            ActorKind::Effect => Self::Effect,
            ActorKind::Job => {
                return Err(DomainError::Invalid(
                    "a job cannot emit a command proposal".into(),
                ))
            }
            ActorKind::Component => {
                return Err(DomainError::Invalid(
                    "a component cannot emit a command proposal: it acts for a person".into(),
                ))
            }
        })
    }
}

/// How a command run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    Ok,
    Denied,
    Invalid,
    Error,
}

/// Version 6 names the command detail row by its command run ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandExecutedV6 {
    /// The command's name (`oxplow.thread.mark`, `oxplow.config.set`).
    pub command: String,
    pub actor_kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub outcome: CommandOutcome,
    /// The `command_run` row, which holds the input and the inverse.
    pub command_run_id: i64,
    /// Whether an inverse was recorded, i.e. `commands.undo` can apply.
    pub undoable: bool,
}

pub struct CommandExecuted;
impl EventType for CommandExecuted {
    const TYPE: &'static str = "command.executed";
    const V: u32 = 6;
    type Payload = CommandExecutedV6;
}

/// `command.proposed@4`: an agent or an effect ran a command that needs a
/// person's confirmation; it is kept as a proposal until a person decides.
/// v4 names a page among the actor kinds where v3 named a lens (V114
/// rewrote v3 events).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandProposedV4 {
    /// The proposal's id.
    pub proposal: i64,
    /// The command's name (`oxplow.config.set`).
    pub command: String,
    pub actor_kind: ActorKindV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Whether the command is destructive.
    pub destructive: bool,
    /// The ids of the pending proposals of the same call this one replaced —
    /// marked superseded in the same transaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<i64>,
}

pub struct CommandProposed;
impl EventType for CommandProposed {
    const TYPE: &'static str = "command.proposed";
    const V: u32 = 4;
    type Payload = CommandProposedV4;
}

/// `command.approved@3`: a person approved a proposal and the command ran
/// as them. Its detail row is `command_run_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandApprovedV3 {
    /// The proposal's id.
    pub proposal: i64,
    pub command: String,
    /// The approving run's `command_run` row.
    pub command_run_id: i64,
}

pub struct CommandApproved;
impl EventType for CommandApproved {
    const TYPE: &'static str = "command.approved";
    const V: u32 = 3;
    type Payload = CommandApprovedV3;
}

/// `command.declined@2`: a person declined a proposal; nothing ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandDeclinedV2 {
    /// The proposal's id.
    pub proposal: i64,
    pub command: String,
}

pub struct CommandDeclined;
impl EventType for CommandDeclined {
    const TYPE: &'static str = "command.declined";
    const V: u32 = 2;
    type Payload = CommandDeclinedV2;
}

/// `config.changed@3`: one registered setting changed at an explicit level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigChangedV3 {
    pub key: String,
    pub before: Value,
    pub after: Value,
    pub layer: crate::settings::Level,
}

pub struct ConfigChanged;
impl EventType for ConfigChanged {
    const TYPE: &'static str = "config.changed";
    const V: u32 = 3;
    type Payload = ConfigChangedV3;
    fn upcast(from_v: u32, mut payload: Value) -> Result<Value, DomainError> {
        if from_v != 2 {
            return Err(DomainError::Invalid(format!(
                "config.changed@{from_v} has no upcast"
            )));
        }
        if payload.get("layer").and_then(Value::as_str) == Some("personal") {
            payload["layer"] = Value::String("local".into());
        }
        Ok(payload)
    }
}

/// `file.saved@1`: a person saved a file of a stream's worktree from the
/// editor (`oxplow.file.save`) — what something that should happen on save
/// reacts to (an effect `on: [file.saved]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSavedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
    /// How many bytes it holds now.
    pub bytes: u64,
}

pub struct FileSaved;
impl EventType for FileSaved {
    const TYPE: &'static str = "file.saved";
    const V: u32 = 1;
    type Payload = FileSavedV1;
}

/// `file.created@1`: a file was made in a stream's worktree
/// (`oxplow.file.create`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileCreatedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
}

pub struct FileCreated;
impl EventType for FileCreated {
    const TYPE: &'static str = "file.created";
    const V: u32 = 1;
    type Payload = FileCreatedV1;
}

/// `terminal.opened@1`: a terminal was opened in a stream
/// (`oxplow.terminal.open`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalOpenedV1 {
    /// The stream (`str2`) whose worktree it runs in.
    pub stream: String,
    /// The terminal (`trm3`).
    pub terminal: String,
    pub title: String,
}

pub struct TerminalOpened;
impl EventType for TerminalOpened {
    const TYPE: &'static str = "terminal.opened";
    const V: u32 = 1;
    type Payload = TerminalOpenedV1;
}

/// `terminal.renamed@1`: a terminal got another title
/// (`oxplow.terminal.rename`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalRenamedV1 {
    pub stream: String,
    /// The terminal (`trm3`).
    pub terminal: String,
    pub title: String,
}

pub struct TerminalRenamed;
impl EventType for TerminalRenamed {
    const TYPE: &'static str = "terminal.renamed";
    const V: u32 = 1;
    type Payload = TerminalRenamedV1;
}

/// `terminal.closed@1`: a terminal was closed: its shell ended and its row
/// went (`oxplow.terminal.close`, or its stream's archive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalClosedV1 {
    pub stream: String,
    /// The terminal (`trm3`).
    pub terminal: String,
    pub title: String,
}

pub struct TerminalClosed;
impl EventType for TerminalClosed {
    const TYPE: &'static str = "terminal.closed";
    const V: u32 = 1;
    type Payload = TerminalClosedV1;
}

/// `file.directory_created@1`: a directory was made in a stream's worktree
/// (`oxplow.file.create_directory`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDirectoryCreatedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
}

pub struct FileDirectoryCreated;
impl EventType for FileDirectoryCreated {
    const TYPE: &'static str = "file.directory_created";
    const V: u32 = 1;
    type Payload = FileDirectoryCreatedV1;
}

/// `file.renamed@1`: a file or directory of a stream's worktree moved
/// (`oxplow.file.rename`). What holds the old path — an open editor
/// buffer, a tab — follows it to the new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRenamedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// Where it was.
    pub from: String,
    /// Where it is.
    pub to: String,
}

pub struct FileRenamed;
impl EventType for FileRenamed {
    const TYPE: &'static str = "file.renamed";
    const V: u32 = 1;
    type Payload = FileRenamedV1;
}

/// `file.deleted@1`: a file or directory of a stream's worktree was
/// removed (`oxplow.file.delete`). What holds it — an open editor buffer
/// under the path — closes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDeletedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// The path that is gone.
    pub path: String,
}

pub struct FileDeleted;
impl EventType for FileDeleted {
    const TYPE: &'static str = "file.deleted";
    const V: u32 = 1;
    type Payload = FileDeletedV1;
}

/// `capability.switched@3`: a switch, with the level the choice came from
/// (`local`, `project`, `global`) or why none did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySwitchedV3 {
    pub capability: String,
    pub from: String,
    pub to: String,
    pub chosen_by: crate::capability::ChosenBy,
}
pub struct CapabilitySwitched;
impl EventType for CapabilitySwitched {
    const TYPE: &'static str = "capability.switched";
    const V: u32 = 3;
    type Payload = CapabilitySwitchedV3;
    /// @1 and @2 named the local level `personal`.
    fn upcast(from_v: u32, mut payload: Value) -> Result<Value, DomainError> {
        if !matches!(from_v, 1 | 2) {
            return Err(DomainError::Invalid(format!(
                "capability.switched@{from_v} has no upcast"
            )));
        }
        if payload.get("chosen_by").and_then(Value::as_str) == Some("personal") {
            payload["chosen_by"] = Value::String("local".into());
        }
        Ok(payload)
    }
}

/// How an effect's reaction to one event ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EffectOutcome {
    /// Its commands ran.
    Ok,
    /// Its script decided there was nothing to do.
    Skipped,
    /// A command it composed needs a person's confirmation: kept as a
    /// proposal.
    Proposed,
    /// It failed (or was interrupted), and its steps aren't sent again.
    Failed,
}

/// What started an attempt at a reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EffectOrigin {
    /// The pump delivering the event.
    Live,
    /// A person's `effect.retry` of a reaction that failed.
    Retry,
    /// A person's `effect.backfill` over events the effect never saw.
    Backfill,
    /// Sent again by itself (P10): a failed attempt whose every step went
    /// to a provider that keeps `idempotent_writes`.
    Auto,
}

/// `effect.result@5` (P10): v3, its origin able to say `auto` — an
/// attempt sent again by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectResultV5 {
    /// The effect (`effect:<extension>/<id>`).
    pub effect: String,
    /// The id of the event it reacted to (a UUID); absent on a v1 result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    pub outcome: EffectOutcome,
    /// Why it skipped or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The id of the proposal it left for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<i64>,
    /// Effect-specific detail, opaque to the log.
    #[serde(default)]
    pub detail: Value,
    /// Which attempt at the reaction, from 1.
    pub attempt: u32,
    pub origin: EffectOrigin,
}

pub struct EffectResult;
impl EventType for EffectResult {
    const TYPE: &'static str = "effect.result";
    const V: u32 = 5;
    type Payload = EffectResultV5;
}

/// `snapshot.taken@3`: v1, its trigger able to say
/// `run_measured` — the take that pins a run's reports to the code it
/// measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotTakenV3 {
    /// The stream (`str1`).
    pub stream: String,
    /// The snapshot the worktree is at after the take: a new one, or the
    /// parent when nothing changed (`unchanged`).
    pub snapshot: i64,
    /// The snapshot it was taken from, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    pub trigger: crate::snapshot::SnapshotTrigger,
    /// True when the take recorded no new snapshot.
    pub unchanged: bool,
    /// File rows the take recorded (0 when unchanged).
    pub file_count: u32,
    pub elapsed_ms: u64,
    /// The time budget the caller gave the take, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_ms: Option<u64>,
    /// `elapsed_ms > budget_ms`: reported, never silent.
    pub over_budget: bool,
}

pub struct SnapshotTaken;
impl EventType for SnapshotTaken {
    const TYPE: &'static str = "snapshot.taken";
    const V: u32 = 3;
    type Payload = SnapshotTakenV3;
}

/// `vcs.head.moved@3`: a stream's HEAD moved (a commit, checkout, merge,
/// reset in its worktree), whether the tree was clean or not. Logged once
/// per move, after the take that follows it. A ref moving elsewhere in the
/// repository is not one. Collectors that follow commits (the
/// `where`-less `on: [vcs.head.moved]`) run on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VcsHeadMovedV3 {
    /// The stream (`str1`).
    pub stream: String,
    /// The snapshot re-stamped with the new commit; absent when the tree was
    /// not clean, so the next take records the commit instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<i64>,
    /// The commit (its sha) HEAD pointed at before, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The commit (its sha) HEAD points at now; absent before the first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

pub struct VcsHeadMoved;
impl EventType for VcsHeadMoved {
    const TYPE: &'static str = "vcs.head.moved";
    const V: u32 = 3;
    type Payload = VcsHeadMovedV3;
    fn upcast(from_v: u32, payload: Value) -> Result<Value, DomainError> {
        match from_v {
            // v2 is v3 with a snapshot and a commit always present.
            2 => Ok(payload),
            _ => Err(DomainError::Invalid(format!(
                "vcs.head.moved@{from_v} has no upcast"
            ))),
        }
    }
}

/// `vcs.commit.indexed@2`: the commit indexer stored a commit it hadn't
/// seen, reachable from a stream's head — every commit, however it was made
/// (a `vcs.head.moved` needs a clean worktree, so it misses most).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VcsCommitIndexedV2 {
    /// The commit's sha.
    pub commit: String,
    /// The stream whose workspace it was found from (`str1`).
    pub stream: String,
    /// When it was committed (RFC 3339).
    pub committed_at: String,
}

pub struct VcsCommitIndexed;
impl EventType for VcsCommitIndexed {
    const TYPE: &'static str = "vcs.commit.indexed";
    const V: u32 = 2;
    type Payload = VcsCommitIndexedV2;
}

/// `agent.turn.started@3`: a turn opened — by a person's prompt, a
/// background job reporting back, a subagent's hand-back or another
/// session's message (`source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnStartedV3 {
    /// The turn (`trn12`).
    pub turn: String,
    /// The thread (`thr3`).
    pub thread: String,
    /// The harness session id, when it reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// What opened it. Turns logged before sources were read `unknown`.
    pub source: crate::hook::TurnSource,
    /// The turn whose work this one goes on with, when the harness named
    /// it: the one whose tool call started the background job, or in
    /// which the subagent started (`trn11`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause_turn: Option<String>,
}

pub struct AgentTurnStarted;
impl EventType for AgentTurnStarted {
    const TYPE: &'static str = "agent.turn.started";
    const V: u32 = 3;
    type Payload = AgentTurnStartedV3;
}

/// A turn's token counts, when the harness reported them with the turn
/// (the Codex observer's per-turn report). Claude's come from the transcript instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// `agent.turn.ended@5`: a turn closed — how (`outcome`), what it left
/// running, and what the token-usage reactor needs to count it (the
/// transcript to read, or the counts themselves).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnEndedV5 {
    /// The turn (`trn12`).
    pub turn: String,
    /// The thread (`thr3`).
    pub thread: String,
    pub outcome: crate::hook::TurnOutcome,
    /// The harness's session transcript (Claude's `transcript_path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// Counts reported with the turn (the Codex observer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    /// The jobs and subagents still running when it ended, when the
    /// harness said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_running: Option<Vec<crate::hook::RunningWork>>,
    /// The harness's own words for the error that ended it (`outcome:
    /// errored`): `name: message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct AgentTurnEnded;
impl EventType for AgentTurnEnded {
    const TYPE: &'static str = "agent.turn.ended";
    const V: u32 = 5;
    type Payload = AgentTurnEndedV5;
}

/// Where a large or sensitive body lives: `event_content.hash` (xxh3-128
/// hex) and its length. Retention may delete the body; the ref stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    pub hash: String,
    /// The whole body's length, stored or not.
    pub size: u64,
    /// Only the first part was stored (bodies are capped).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// A status a thread's agent can be logged in. There is no `stalled`: that
/// is derived from silence and never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoggedAgentStatus {
    Idle,
    Running,
    /// Parked on the person: a question or plan put to them, a permission
    /// prompt.
    AwaitingUser,
    Stopped,
    Error,
}

impl LoggedAgentStatus {
    /// The loggable form of `state`; `None` for a derived-only state.
    pub fn of(state: crate::hook::AgentStatusState) -> Option<Self> {
        use crate::hook::AgentStatusState as S;
        match state {
            S::Idle => Some(Self::Idle),
            S::Running => Some(Self::Running),
            S::AwaitingUser => Some(Self::AwaitingUser),
            S::Stopped => Some(Self::Stopped),
            S::Error => Some(Self::Error),
            S::Stalled => None,
        }
    }
}

impl From<LoggedAgentStatus> for crate::hook::AgentStatusState {
    fn from(s: LoggedAgentStatus) -> Self {
        use crate::hook::AgentStatusState as S;
        match s {
            LoggedAgentStatus::Idle => S::Idle,
            LoggedAgentStatus::Running => S::Running,
            LoggedAgentStatus::AwaitingUser => S::AwaitingUser,
            LoggedAgentStatus::Stopped => S::Stopped,
            LoggedAgentStatus::Error => S::Error,
        }
    }
}

/// `agent.session.started@3`: v1 with the harness as its registry key
/// (`claude`, or whatever an extension declares). A v1 payload is a v2 one
/// as is: its four names are those keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionStartedV3 {
    pub session: String,
    /// The thread (`thr3`).
    pub thread: String,
    /// The harness's registry key; empty when no agent session claims the
    /// hook (an agent oxplow didn't start).
    pub harness: String,
    /// The session is the thread's resume session (a reattach), not new.
    pub resumed: bool,
}

pub struct AgentSessionStarted;
impl EventType for AgentSessionStarted {
    const TYPE: &'static str = "agent.session.started";
    const V: u32 = 3;
    type Payload = AgentSessionStartedV3;
}

/// `agent.session.ended@2`: the harness ended a session (`/clear`, exit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionEndedV2 {
    pub session: String,
    /// The thread (`thr3`).
    pub thread: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub struct AgentSessionEnded;
impl EventType for AgentSessionEnded {
    const TYPE: &'static str = "agent.session.ended";
    const V: u32 = 2;
    type Payload = AgentSessionEndedV2;
}

/// A kind of token an agent's telemetry counts. Cache kinds are their
/// own: a sum over input and output must never include them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Input,
    Output,
    CacheRead,
    CacheCreation,
}

impl TokenKind {
    /// The conformed `oxplow.token_kind` dimension value.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Input => "input",
            TokenKind::Output => "output",
            TokenKind::CacheRead => "cache_read",
            TokenKind::CacheCreation => "cache_creation",
        }
    }

    /// Whether it is a cache kind (counted on its own measure).
    pub fn is_cache(self) -> bool {
        matches!(self, TokenKind::CacheRead | TokenKind::CacheCreation)
    }
}

/// One count an agent's telemetry reported: `value` tokens of `kind` for
/// `model`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenCount {
    pub model: String,
    pub kind: TokenKind,
    pub value: u64,
}

/// `agent.tokens.reported@2`: an agent's own telemetry (an OTLP export)
/// reported token counts. Anchored to the turn the export's time window
/// fell in — exports lag the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTokensReportedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    pub counts: Vec<TokenCount>,
    /// The end of the export's time window (RFC 3339), when it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end: Option<String>,
}

pub struct AgentTokensReported;
impl EventType for AgentTokensReported {
    const TYPE: &'static str = "agent.tokens.reported";
    const V: u32 = 2;
    type Payload = AgentTokensReportedV2;
}

/// `agent.prompt.submitted@3`: a prompt reached the agent — a person's,
/// a background job's report, a subagent's hand-back, another session's
/// message (`source`). Every delivery logs one; one with no turn open also
/// opens a turn (`reprompt` says it landed inside an open one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPromptSubmittedV3 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn it opened, or the one it was delivered into (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub reprompt: bool,
    /// Who it came from. Rows logged before sources were read `unknown`.
    pub source: crate::hook::TurnSource,
    /// The text the harness delivered, stored in `event_content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<ContentRef>,
    /// On another session's message: the sender's own name for its
    /// session (its socket when it has no name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// On a background job's report or a subagent's hand-back: the
    /// harness's id for the job (the subagent's id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}

pub struct AgentPromptSubmitted;
impl EventType for AgentPromptSubmitted {
    const TYPE: &'static str = "agent.prompt.submitted";
    const V: u32 = 3;
    type Payload = AgentPromptSubmittedV3;
}

/// What oxplow's agent policy said about a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    Allowed,
    Denied,
}

/// A subagent inside an agent session, as a tool event or a subagent
/// event names it: not a session of its own, but who in the session acted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubagentRef {
    /// The harness's id for the subagent, or the id of the call that
    /// started it when the harness gives none.
    pub id: String,
    /// What kind of subagent the harness says it is, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `agent.tool.requested@3`: the agent asked to run a tool (PreToolUse),
/// and whether the policy let it. As `agent.tool.requested@2`, plus the
/// subagent that asked, when it wasn't the session's own agent. v2 upcasts
/// as is (no subagent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentToolRequestedV4 {
    pub tool: String,
    pub kind: crate::agent::tool::ToolKind,
    /// The harness's id for the call, the same on its finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// The files it names, repo-relative inside the worktree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// A short summary (a truncated command, a search pattern).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ContentRef>,
    pub decision: ToolDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The subagent that asked, when it wasn't the session's own agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentRef>,
}

pub struct AgentToolRequested;
impl EventType for AgentToolRequested {
    const TYPE: &'static str = "agent.tool.requested";
    const V: u32 = 4;
    type Payload = AgentToolRequestedV4;
}

/// `agent.tool.finished@4`: a tool call returned (PostToolUse), caused by
/// its `agent.tool.requested` when one was logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentToolFinishedV4 {
    pub tool: String,
    pub kind: crate::agent::tool::ToolKind,
    /// The harness's id for the call, the same on its request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// How long it ran, in milliseconds, as the harness timed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A shell call's command line, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Whether it succeeded, when the harness said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// A shell command's exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ContentRef>,
    /// The subagent that made the call, when it wasn't the session's own
    /// agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentRef>,
}

pub struct AgentToolFinished;
impl EventType for AgentToolFinished {
    const TYPE: &'static str = "agent.tool.finished";
    const V: u32 = 4;
    type Payload = AgentToolFinishedV4;
}

/// `agent.subagent.started@2`: a subagent of the session started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSubagentStartedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    pub subagent: SubagentRef,
}

pub struct AgentSubagentStarted;
impl EventType for AgentSubagentStarted {
    const TYPE: &'static str = "agent.subagent.started";
    const V: u32 = 2;
    type Payload = AgentSubagentStartedV2;
}

/// `agent.subagent.finished@2`: a subagent of the session finished — it
/// stopped, or handed its report back. One per subagent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSubagentFinishedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    pub subagent: SubagentRef,
    /// What it said last (Claude's `SubagentStop.last_assistant_message`),
    /// stored in `event_content`; read as `subagent:<id>#result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ContentRef>,
}

pub struct AgentSubagentFinished;
impl EventType for AgentSubagentFinished {
    const TYPE: &'static str = "agent.subagent.finished";
    const V: u32 = 2;
    type Payload = AgentSubagentFinishedV2;
}

/// `agent.context.compacted@2`: the agent's context was summarized to make
/// room, so what it was told earlier is no longer in front of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentContextCompactedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// What started it, as the harness names it (Claude's `auto` or
    /// `manual`), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// The summary the context was replaced with (Claude's
    /// `compact_summary`), stored in `event_content`; read as
    /// `oxplow:oxplow/event?id=<id>#part=summary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ContentRef>,
}

pub struct AgentContextCompacted;
impl EventType for AgentContextCompacted {
    const TYPE: &'static str = "agent.context.compacted";
    const V: u32 = 2;
    type Payload = AgentContextCompactedV2;
}

/// `agent.status.changed@2`: a thread's agent moved to another state.
/// Logged on a transition only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusChangedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    pub state: LoggedAgentStatus,
    /// What it's waiting on (the question, the permission asked), or why
    /// it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub struct AgentStatusChanged;
impl EventType for AgentStatusChanged {
    const TYPE: &'static str = "agent.status.changed";
    const V: u32 = 2;
    type Payload = AgentStatusChangedV2;
}

/// `test.run.recorded@3`: a test run was captured (`oxplow:oxplow/run?id=<capture>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRunRecordedV3 {
    /// The run's id (its capture).
    pub run: i64,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    /// A test report was found and parsed (else the run is command-only).
    pub report_parsed: bool,
    /// Who produced it: `post-tool-bash`, `exec:<collector ids>` (an `exec`
    /// collector's program parsed its report), `mcp`.
    pub source: String,
}

pub struct TestRunRecorded;
impl EventType for TestRunRecorded {
    const TYPE: &'static str = "test.run.recorded";
    const V: u32 = 3;
    type Payload = TestRunRecordedV3;
}

/// `test.coverage.recorded@1`: a coverage report was captured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestCoverageRecordedV1 {
    /// The `metric_capture` id.
    pub capture: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches_pct: Option<f64>,
    pub source: String,
}

pub struct TestCoverageRecorded;
impl EventType for TestCoverageRecorded {
    const TYPE: &'static str = "test.coverage.recorded";
    const V: u32 = 1;
    type Payload = TestCoverageRecordedV1;
}

/// `collector.synced@2` (P7.B3): a collector ran — by hand
/// (`oxplow.collector.sync`), on its schedule, or for an event its `on:` trigger
/// names (then the envelope's `cause` is that event). Logged in the
/// transaction that wrote what it collected; a run that failed logs one
/// too, with what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectorSyncedV2 {
    /// The collector (`<extension>/<id>`).
    pub collector: String,
    /// What ran it: `manual`, `every` or `on`.
    pub trigger: String,
    /// `ok` or `error`.
    pub status: String,
    /// Rows per entity after the run (a failed run wrote none).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entities: BTreeMap<String, i64>,
    /// Facts it recorded.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub facts: i64,
    pub elapsed_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

pub struct CollectorSynced;
impl EventType for CollectorSynced {
    const TYPE: &'static str = "collector.synced";
    const V: u32 = 2;
    type Payload = CollectorSyncedV2;
}

/// Why a thread reached a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    /// A turn ended and its snapshot was taken.
    TurnEnd,
}

/// `thread.checkpoint@2`: a point in a thread's work oxplow observed, with
/// what changed since the turn began — what a reactor reacts to without
/// reading snapshots or harness tool names
/// (`.context/work-tracking.md`). Logged by the `thread.checkpoint`
/// consumer once a turn's end take lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadCheckpointV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn (`trn12`).
    pub turn: String,
    pub reason: CheckpointReason,
    /// The snapshot of the worktree at the checkpoint.
    pub snapshot: i64,
    /// The worktree differs from where the turn began.
    pub changed: bool,
    /// The turn's calls to tools that can change the worktree (edits,
    /// shell commands, subagents, oxplow commands); 0 for a turn that only
    /// read and answered.
    pub writing_tools: u32,
}

pub struct ThreadCheckpoint;
impl EventType for ThreadCheckpoint {
    const TYPE: &'static str = "thread.checkpoint";
    const V: u32 = 2;
    type Payload = ThreadCheckpointV2;
}

/// How many diagnostics of each severity a file has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticCounts {
    pub error: u32,
    pub warning: u32,
    pub information: u32,
    pub hint: u32,
}

/// `code.diagnostics.changed@2`: what a file's language servers report
/// about it changed (a publish, or a server gone and its reports with
/// it). Logged debounced, once per file per burst, with the counts after
/// it; `v_diagnostic` has the rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeDiagnosticsChangedV2 {
    /// The stream (`str1`).
    pub stream: String,
    /// Workspace-relative.
    pub path: String,
    pub counts: DiagnosticCounts,
}

pub struct CodeDiagnosticsChanged;
impl EventType for CodeDiagnosticsChanged {
    const TYPE: &'static str = "code.diagnostics.changed";
    const V: u32 = 2;
    type Payload = CodeDiagnosticsChangedV2;
}

/// `contribution.disabled@1` (P7.C1): one of an extension's contributions —
/// a provider instance, a collector, an effect — was stopped on this machine after
/// repeated failures (or a provider whose handshake no longer matches what
/// was approved), and stays off until a person enables it again
/// (`oxplow.contribution.enable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContributionDisabledV1 {
    /// The contribution (`oxplow:oxplow/collector?collector=<extension>/<id>`, `effect:…`,
    /// `provider:…`).
    pub contribution: String,
    /// What stopped it.
    pub reason: String,
}

pub struct ContributionDisabled;
impl EventType for ContributionDisabled {
    const TYPE: &'static str = "contribution.disabled";
    const V: u32 = 1;
    type Payload = ContributionDisabledV1;
}

/// `contribution.enabled@1` (P7.C1): a person enabled a disabled contribution
/// again on this machine (`oxplow.contribution.enable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContributionEnabledV1 {
    /// The contribution (`oxplow:oxplow/collector?collector=<extension>/<id>`, `effect:…`,
    /// `provider:…`).
    pub contribution: String,
}

pub struct ContributionEnabled;
impl EventType for ContributionEnabled {
    const TYPE: &'static str = "contribution.enabled";
    const V: u32 = 1;
    type Payload = ContributionEnabledV1;
}

/// How an agent read a knowledge page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadVia {
    /// Through a tool (`oxplow_read`).
    Tool,
    /// Its own read of the page's file (a store whose pages are files).
    File,
}

impl ReadVia {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadVia::Tool => "tool",
            ReadVia::File => "file",
        }
    }
}

/// `record.read@1`: an agent read a row of a shared model — through
/// `oxplow_read` (`via: tool`), or its own read of the file the row's store
/// keeps it in (`via: file`, caused by the tool call; the store's `files`
/// declaration in its `fills`). Whatever keeps reads of a model's rows follows
/// this event; a person's opens are their page visits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordReadV1 {
    /// The row (`oxplow:oxplow/knowledge?id=<id>&store=<store>`).
    pub record: String,
    pub via: ReadVia,
}

pub struct RecordRead;
impl EventType for RecordRead {
    const TYPE: &'static str = "record.read";
    const V: u32 = 1;
    type Payload = RecordReadV1;
}

/// `snapshot.ref_drifted@1`: a snapshot of the primary stream changed files a
/// source's pinned refs cite (`file_pin` / `dir_pin` edges, a shared
/// model's pinned markdown column), so what it says of them may no longer
/// hold (`v_ref_pin.stale`), whether or not it was stale already. Caused by
/// the `snapshot.taken`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefDriftedV1 {
    /// The source that pins them (a model row's URI).
    pub source: String,
    /// The snapshot that changed them.
    pub snapshot: i64,
    /// The changed files it cites, or that are under a directory it cites
    /// (repo-relative).
    pub paths: Vec<String>,
}

pub struct RefDrifted;
impl EventType for RefDrifted {
    const TYPE: &'static str = "snapshot.ref_drifted";
    const V: u32 = 1;
    type Payload = RefDriftedV1;
}

/// `ui.op_failed@3`: an operation the person started in the app failed, as
/// the app showed it (`oxplow.ui.report_error`). Its captured stderr and
/// stdout are the `output` body. The event can link to a command run and
/// its approval proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOpFailedV3 {
    /// What the person was doing ("Merge bugfixes into current").
    pub label: String,
    /// The command it ran, shell-style ("git merge bugfixes").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The error message, when there was no captured output to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The process's exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// The thread it was started from (`thr3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The signal that killed the process (`SIGKILL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// How long it ran, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Its captured output, `{ stderr?, stdout? }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ContentRef>,
    /// A command whose run failed: its detail row (`command_run.id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_run_id: Option<i64>,
    /// An approval that failed: its proposal (`command_proposal.id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<i64>,
}

pub struct UiOpFailed;
impl EventType for UiOpFailed {
    const TYPE: &'static str = "ui.op_failed";
    const V: u32 = 3;
    type Payload = UiOpFailedV3;
    fn upcast(from_v: u32, payload: Value) -> Result<Value, DomainError> {
        match from_v {
            // v1 has neither link.
            1 => Ok(payload),
            _ => Err(DomainError::Invalid(format!(
                "ui.op_failed@{from_v} has no upcast"
            ))),
        }
    }
}

/// The failed operation a person's mark is about: a `ui.op_failed` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOpErrorMarkV1 {
    /// The `ui.op_failed` event (`v_op_error.event_id`).
    pub event: String,
}

/// `ui.op_error.seen@1`: the person opened a failed operation's output
/// (`oxplow.op_error.mark_seen`); `v_op_error.seen` reads it.
pub struct UiOpErrorSeen;
impl EventType for UiOpErrorSeen {
    const TYPE: &'static str = "ui.op_error.seen";
    const V: u32 = 1;
    type Payload = UiOpErrorMarkV1;
}

/// `ui.op_error.dismissed@1`: the person dismissed a failed operation
/// (`oxplow.op_error.dismiss`, `dismiss_all`); `v_op_error.dismissed`
/// reads it.
pub struct UiOpErrorDismissed;
impl EventType for UiOpErrorDismissed {
    const TYPE: &'static str = "ui.op_error.dismissed";
    const V: u32 = 1;
    type Payload = UiOpErrorMarkV1;
}

/// `thread.closed@1`: a thread was closed (`oxplow.thread.close`): its
/// sessions ended with it, and what it was working on goes back to nobody
/// (bundled releases its tasks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadClosedV1 {
    /// The thread (`thr3`).
    pub thread: String,
}

pub struct ThreadClosed;
impl EventType for ThreadClosed {
    const TYPE: &'static str = "thread.closed";
    const V: u32 = 1;
    type Payload = ThreadClosedV1;
}

/// `change.analyzed@1`: a change's analysis results were stored (phase
/// `files`) or its duplicate scan was stored (phase `duplicates`). An
/// analysis whose inputs didn't move logs nothing. A collector that
/// follows changes (`on: [change.analyzed]`, `where: { phase: files }`)
/// runs on it, anchored to the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeAnalyzedV1 {
    /// The change (`v_change.id`).
    pub change_id: i64,
    /// `commit`, `turn`, `request`, `working` or `range`.
    pub kind: String,
    /// The sha, turn or request id, `<base>..<head>`; '' for the working tree.
    pub target: String,
    /// The stream (`v_change.stream_id`).
    pub stream_id: i64,
    /// The older side (`working`, `snap:<id>`, `git:<sha>`); absent for a
    /// root commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    /// The newer side, spelled like `base_revision`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_revision: Option<String>,
    /// `files` when the analysis is stored, `duplicates` when its
    /// duplicate scan is.
    pub phase: String,
}

pub struct ChangeAnalyzed;
impl EventType for ChangeAnalyzed {
    const TYPE: &'static str = "change.analyzed";
    const V: u32 = 1;
    type Payload = ChangeAnalyzedV1;
}

/// `stream.archived@1`: a stream was archived (`oxplow.stream.archive`):
/// its threads' sessions ended and its working copy may be gone; what its
/// threads were working on goes back to nobody.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamArchivedV1 {
    /// The stream (`str2`).
    pub stream: String,
    /// Its threads (`thr3`), each.
    pub threads: Vec<String>,
}

pub struct StreamArchived;
impl EventType for StreamArchived {
    const TYPE: &'static str = "stream.archived";
    const V: u32 = 1;
    type Payload = StreamArchivedV1;
}

/// Whether `event_type` is one of core's types (any version) — what a
/// collector's `on:` trigger may name besides its own extension's
/// declared types. Built once.
pub fn is_core_type(event_type: &str) -> bool {
    static CORE: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    CORE.get_or_init(|| {
        EventSchemaRegistry::core()
            .versions()
            .into_iter()
            .map(|(t, _)| t)
            .collect()
    })
    .contains(event_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn core_registers_only_each_types_newest_version() {
        // A core bump rewrites the logged rows in a migration and deletes
        // the old version; only an extension's declared types keep theirs.
        let r = EventSchemaRegistry::core();
        let mut types: Vec<String> = r.versions().into_iter().map(|(t, _)| t).collect();
        let all = types.len();
        types.dedup();
        assert_eq!(types.len(), all, "a core type registers an old version");
    }

    #[test]
    fn core_registry_knows_every_core_type_and_version() {
        let r = EventSchemaRegistry::core();
        let all = r.versions();
        let versions: Vec<(&str, u32)> = all.iter().map(|(t, v)| (t.as_str(), *v)).collect();
        assert_eq!(
            versions,
            vec![
                ("agent.context.compacted", 2),
                ("agent.prompt.submitted", 3),
                ("agent.session.ended", 2),
                ("agent.session.started", 3),
                ("agent.status.changed", 2),
                ("agent.subagent.finished", 2),
                ("agent.subagent.started", 2),
                ("agent.tokens.reported", 2),
                ("agent.tool.finished", 4),
                ("agent.tool.requested", 4),
                ("agent.turn.ended", 5),
                ("agent.turn.started", 3),
                ("capability.switched", 3),
                ("change.analyzed", 1),
                ("code.diagnostics.changed", 2),
                ("collector.synced", 2),
                ("command.approved", 3),
                ("command.declined", 2),
                ("command.executed", 6),
                ("command.proposed", 4),
                ("config.changed", 3),
                ("contribution.disabled", 1),
                ("contribution.enabled", 1),
                ("contribution.job.completed", 2),
                ("effect.result", 5),
                ("file.created", 1),
                ("file.deleted", 1),
                ("file.directory_created", 1),
                ("file.renamed", 1),
                ("file.saved", 1),
                ("record.read", 1),
                ("snapshot.ref_drifted", 1),
                ("snapshot.taken", 3),
                ("stream.archived", 1),
                ("terminal.closed", 1),
                ("terminal.opened", 1),
                ("terminal.renamed", 1),
                ("test.coverage.recorded", 1),
                ("test.run.recorded", 3),
                ("thread.checkpoint", 2),
                ("thread.closed", 1),
                ("ui.op_error.dismissed", 1),
                ("ui.op_error.seen", 1),
                ("ui.op_failed", 3),
                ("vcs.commit.indexed", 2),
                ("vcs.head.moved", 3),
            ]
        );
        assert_eq!(r.latest("thread.closed"), Some(1));
        assert_eq!(r.latest("agent.turn.ended"), Some(5));
        assert_eq!(r.owner("config.changed", 3), Some(None));
    }

    /// A switch logged when the local level was called `personal` reads
    /// as `local`.
    #[test]
    fn an_older_capability_switch_upcasts_personal_to_local() {
        let r = EventSchemaRegistry::core();
        for from in [1, 2] {
            let (v, payload) = r
                .upcast_to_latest(
                    "capability.switched",
                    from,
                    serde_json::json!({ "capability": "snapshots", "from": "oxplow", "to": "hashes", "chosen_by": "personal" }),
                )
                .unwrap();
            assert_eq!(v, 3);
            assert_eq!(payload["chosen_by"], "local");
        }
    }

    #[test]
    fn validate_accepts_the_typed_shape_and_names_the_violation() {
        let r = crate::vocabulary::Vocabulary::core();
        let ok = Envelope::typed::<ThreadClosed>(
            "human",
            &ThreadClosedV1 {
                thread: "thr3".into(),
            },
        );
        assert_eq!(ok.event_type, "thread.closed");
        assert_eq!(ok.v, 1);
        r.validate_envelope(&ok).unwrap();

        let bad = r
            .validate("thread.closed", 1, &json!({"thread": 7}))
            .unwrap_err();
        let msg = bad.to_string();
        assert!(
            msg.contains("thread.closed@1") && msg.contains("/thread"),
            "{msg}"
        );

        let extra = r
            .validate(
                "config.changed",
                3,
                &json!({"key": "oxplow.snapshots.retentionDays", "before": null, "after": 1, "layer": "project", "oops": 1}),
            )
            .unwrap_err();
        assert!(extra.to_string().contains("oops"), "{extra}");
    }

    #[test]
    fn unknown_types_and_versions_are_refused() {
        let r = EventSchemaRegistry::core();
        let e = r.validate("thread.closed", 2, &json!({})).unwrap_err();
        assert!(e.to_string().contains("newest is v1"), "{e}");
        let e = r.validate("acme.thing", 1, &json!({})).unwrap_err();
        assert!(e.to_string().contains("not registered"), "{e}");
    }

    /// A declared type with `T`'s schema and no upcast.
    fn declared<T: EventType>() -> DeclaredEventType {
        DeclaredEventType {
            event_type: T::TYPE.into(),
            v: T::V,
            schema: schema_for::<T>(),
            summary: format!("{}, for the test", T::TYPE),
            upcast: None,
        }
    }

    struct AcmeDecided;
    impl EventType for AcmeDecided {
        const TYPE: &'static str = "acme.decided";
        const V: u32 = 1;
        type Payload = ConfigChangedV3;
    }
    struct NamedForTheExtension;
    impl EventType for NamedForTheExtension {
        const TYPE: &'static str = "acme_review.decided";
        const V: u32 = 1;
        type Payload = ConfigChangedV3;
    }
    struct AcmeFiled;
    impl EventType for AcmeFiled {
        const TYPE: &'static str = "acme.filed";
        const V: u32 = 1;
        type Payload = ConfigChangedV3;
    }
    struct Squatter;
    impl EventType for Squatter {
        const TYPE: &'static str = "thread.hijacked";
        const V: u32 = 1;
        type Payload = ConfigChangedV3;
    }

    #[test]
    fn an_extension_declares_types_only_under_its_own_namespace() {
        let mut r = EventSchemaRegistry::core();
        // The namespace is the manifest's `namespace:`, not the name.
        r.register_declared("acme-review", "acme", declared::<AcmeDecided>())
            .unwrap();
        assert_eq!(r.owner("acme.decided", 1), Some(Some("acme-review")));
        assert_eq!(
            r.summary("acme.decided", 1),
            Some("acme.decided, for the test")
        );
        let by_name = r
            .register_declared("acme-review", "acme", declared::<NamedForTheExtension>())
            .unwrap_err();
        assert!(by_name.to_string().contains("`acme.*`"), "{by_name}");
        let core = r
            .register_declared("acme-review", "acme", declared::<Squatter>())
            .unwrap_err();
        assert!(core.to_string().contains("core namespace"), "{core}");
        let foreign = r
            .register_declared(
                "other-extension",
                "other_extension",
                declared::<AcmeFiled>(),
            )
            .unwrap_err();
        assert!(
            foreign.to_string().contains("other_extension.*"),
            "{foreign}"
        );
        // A second extension in the same namespace declares its own types
        // there; it can't declare one the first already did.
        r.register_declared("acme-tracker", "acme", declared::<AcmeFiled>())
            .unwrap();
        assert_eq!(r.owner("acme.filed", 1), Some(Some("acme-tracker")));
        let dup = r
            .register_declared("acme-tracker", "acme", declared::<AcmeDecided>())
            .unwrap_err();
        assert!(dup.to_string().contains("already registered"), "{dup}");
        // Core can't register into an extension's namespace either, and a
        // second registration of the same type@v collides.
        assert!(r.register::<AcmeDecided>().is_err());
        assert!(r.register::<ThreadClosed>().is_err());
    }

    #[test]
    fn a_declared_type_needs_a_schema_that_compiles_and_an_upcast_past_v1() {
        let mut r = EventSchemaRegistry::new();
        let bad = r
            .register_declared(
                "acme",
                "acme",
                DeclaredEventType {
                    schema: json!({"type": "nonsense"}),
                    ..declared::<Thing1>()
                },
            )
            .unwrap_err();
        assert!(bad.to_string().contains("acme.thing@1"), "{bad}");
        let no_upcast = r
            .register_declared("acme", "acme", declared::<Thing2>())
            .unwrap_err();
        assert!(no_upcast.to_string().contains("upcast"), "{no_upcast}");
        let zero = r
            .register_declared(
                "acme",
                "acme",
                DeclaredEventType {
                    v: 0,
                    ..declared::<Thing1>()
                },
            )
            .unwrap_err();
        assert!(zero.to_string().contains("v0"), "{zero}");
        assert!(r.versions().is_empty());
    }

    // A two-version type: v2 renamed `count` to `n`.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct ThingV1 {
        count: u32,
    }
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct ThingV2 {
        n: u32,
        #[serde(default)]
        note: Option<String>,
    }
    struct Thing1;
    impl EventType for Thing1 {
        const TYPE: &'static str = "acme.thing";
        const V: u32 = 1;
        type Payload = ThingV1;
    }
    struct Thing2;
    impl EventType for Thing2 {
        const TYPE: &'static str = "acme.thing";
        const V: u32 = 2;
        type Payload = ThingV2;
        fn upcast(from_v: u32, payload: Value) -> Result<Value, DomainError> {
            match from_v {
                1 => Ok(json!({ "n": payload["count"] })),
                _ => Err(DomainError::Invalid(format!("no upcast from v{from_v}"))),
            }
        }
    }

    #[test]
    fn upcast_carries_an_old_payload_to_the_newest_version_and_validates_it() {
        let mut r = EventSchemaRegistry::new();
        r.register_declared("acme", "acme", declared::<Thing1>())
            .unwrap();
        r.register_declared(
            "acme",
            "acme",
            DeclaredEventType {
                upcast: Some(Arc::new(Thing2::upcast)),
                ..declared::<Thing2>()
            },
        )
        .unwrap();
        assert_eq!(r.latest("acme.thing"), Some(2));
        // Both versions still validate as written.
        r.validate("acme.thing", 1, &json!({"count": 3})).unwrap();
        r.validate("acme.thing", 2, &json!({"n": 3})).unwrap();
        assert!(r.validate("acme.thing", 2, &json!({"count": 3})).is_err());
        let (v, up) = r
            .upcast_to_latest("acme.thing", 1, json!({"count": 3}))
            .unwrap();
        assert_eq!((v, up), (2, json!({"n": 3})));
        let (v, same) = r
            .upcast_to_latest("acme.thing", 2, json!({"n": 5, "note": "x"}))
            .unwrap();
        assert_eq!((v, same), (2, json!({"n": 5, "note": "x"})));
        assert!(r.upcast_to_latest("acme.thing", 3, json!({})).is_err());
        // The default upcast refuses: a type that bumps V must define one.
        assert!(ThreadClosed::upcast(0, json!({})).is_err());
    }
}
