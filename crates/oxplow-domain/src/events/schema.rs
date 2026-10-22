//! Event payload schemas: one Rust type per `type@v`, a registry that
//! validates payloads on append, and the golden-schema discipline that
//! keeps published event shapes from drifting.
//!
//! **Golden schemas.** Every registered core type's JSON Schema is
//! checked in at `crates/oxplow-domain/schemas/events/<type>@<v>.json`.
//! A test regenerates each schema from its Rust type and fails if the
//! file differs: a published `type@v` is a contract consumers (extensions,
//! lenses, the agent) were written against, so a change is a **new
//! version** (`V + 1`, with an `upcast` from the version before), never an
//! edit. Run the test with `OXPLOW_BLESS=1` to write a new golden.
//!
//! **Namespaces.** Core types live in the namespaces of
//! [`CORE_NAMESPACES`] (`.context/data-model.md` "event_log"). An
//! extension declares types (a JSON Schema each, not a Rust type —
//! [`DeclaredEventType`]) only under its own namespace
//! ([`extension_namespace`]); declaring into a core namespace, or under
//! another extension's, is refused.

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
    "snapshot",
    "vcs",
    "effort",
    "work_item",
    "knowledge",
    "handoff",
    "test",
    "code",
    "command",
    "effect",
    "collector",
    "lens",
    "config",
    "provider",
    "contribution",
    "ui",
    "file",
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
        r.register::<EffectResult>().expect("core type registers");
        r.register::<SnapshotTaken>().expect("core type registers");
        r.register::<VcsHeadMoved>().expect("core type registers");
        r.register::<VcsCommitIndexed>()
            .expect("core type registers");
        r.register::<EffortLanded>().expect("core type registers");
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
        r.register::<AgentPeerMessageReceived>()
            .expect("core type registers");
        r.register::<AgentContextCompacted>()
            .expect("core type registers");
        r.register::<AgentStatusChanged>()
            .expect("core type registers");
        r.register::<AgentPlanProposed>()
            .expect("core type registers");
        r.register::<AgentTokensReported>()
            .expect("core type registers");
        r.register::<TestRunRecorded>()
            .expect("core type registers");
        r.register::<TestCoverageRecorded>()
            .expect("core type registers");
        r.register::<WorkItemDeleted>()
            .expect("core type registers");
        r.register::<EffortOpened>().expect("core type registers");
        r.register::<EffortClosed>().expect("core type registers");
        r.register::<EffortLinked>().expect("core type registers");
        r.register::<EffortRetitled>().expect("core type registers");
        r.register::<EffortClaimDismissed>()
            .expect("core type registers");
        r.register::<EffortClaimVerified>()
            .expect("core type registers");
        r.register::<EffortDecisionReviewed>()
            .expect("core type registers");
        r.register::<EffortDecisionsInferred>()
            .expect("core type registers");
        r.register::<EffortFinished>().expect("core type registers");
        r.register::<CollectorSynced>()
            .expect("core type registers");
        r.register::<WorkItemEdited>().expect("core type registers");
        r.register::<WorkItemCreated>()
            .expect("core type registers");
        r.register::<WorkItemLinked>().expect("core type registers");
        r.register::<WorkItemUnlinked>()
            .expect("core type registers");
        r.register::<WorkItemCommented>()
            .expect("core type registers");
        r.register::<WorkItemRecorded>()
            .expect("core type registers");
        r.register::<WorkItemStateChanged>()
            .expect("core type registers");
        r.register::<ThreadCheckpoint>()
            .expect("core type registers");
        r.register::<KnowledgePageWritten>()
            .expect("core type registers");
        r.register::<KnowledgePageDeleted>()
            .expect("core type registers");
        r.register::<KnowledgePageRecorded>()
            .expect("core type registers");
        r.register::<HandoffSent>().expect("core type registers");
        r.register::<HandoffSettled>().expect("core type registers");
        r.register::<KnowledgeNoteWritten>()
            .expect("core type registers");
        r.register::<KnowledgeNoteDeleted>()
            .expect("core type registers");
        r.register::<KnowledgeCommentWritten>()
            .expect("core type registers");
        r.register::<KnowledgeCommentDeleted>()
            .expect("core type registers");
        r.register::<CodeDiagnosticsChanged>()
            .expect("core type registers");
        r.register::<ContributionEnabled>()
            .expect("core type registers");
        r.register::<ContributionDisabled>()
            .expect("core type registers");
        r.register::<ContributionJobCompleted>()
            .expect("core type registers");
        r.register::<LensShown>().expect("core type registers");
        r.register::<LensKept>().expect("core type registers");
        r.register::<CommandProposed>()
            .expect("core type registers");
        r.register::<CommandApproved>()
            .expect("core type registers");
        r.register::<CommandDeclined>()
            .expect("core type registers");
        r.register::<UiOpFailed>().expect("core type registers");
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

    /// Register an extension's declared type. Its namespace must be the
    /// extension's own — never a core namespace, never another
    /// extension's; its schema must compile; past v1 it must carry an
    /// upcast.
    pub fn register_declared(
        &mut self,
        extension: &str,
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
        let own = extension_namespace(extension);
        if ns != own {
            return Err(DomainError::Invalid(format!(
                "extension `{extension}` may only declare types under `{own}.*`, not `{event_type}`"
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
    Lens,
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
    Lens,
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
            ActorKind::Lens => Self::Lens,
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

/// Version 4 adds a custom component among the actors (v3, durable job
/// attribution; V93 rewrote v3 events: each is a v4 one as is).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandExecutedV4 {
    /// The command's name (`oxplow.work_item.transition`, `oxplow.config.set`).
    pub command: String,
    pub actor_kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub outcome: CommandOutcome,
    /// The `command_audit` row, which holds the input and the inverse.
    pub audit_id: i64,
    /// Whether an inverse was recorded, i.e. `commands.undo` can apply.
    pub undoable: bool,
}

pub struct CommandExecuted;
impl EventType for CommandExecuted {
    const TYPE: &'static str = "command.executed";
    const V: u32 = 4;
    type Payload = CommandExecutedV4;
}

/// `command.proposed@3`: an agent or an effect ran a command that needs a
/// person's confirmation; it is kept as a proposal until a person decides
/// — v1, with an effect among the actor kinds (P8.D8). A v1 payload is a
/// v2 one as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandProposedV3 {
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
    const V: u32 = 3;
    type Payload = CommandProposedV3;
}

/// `command.approved@2`: a person approved a proposal and the command ran
/// as them, audited as `audit_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandApprovedV2 {
    /// The proposal's id.
    pub proposal: i64,
    pub command: String,
    /// The approving run's `command_audit` row.
    pub audit_id: i64,
}

pub struct CommandApproved;
impl EventType for CommandApproved {
    const TYPE: &'static str = "command.approved";
    const V: u32 = 2;
    type Payload = CommandApprovedV2;
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

/// Which layer of a project's config a change was to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigLayer {
    /// The project's shared config (`.oxplow/project.yaml`).
    Project,
    /// A person's own layer over it (`.oxplow/personal.yaml`), which git
    /// ignores.
    Personal,
}

/// `config.changed@2`: one config key changed, in one layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigChangedV2 {
    /// The `ConfigKey` (`zones`, `metricRetentionDays`).
    pub key: String,
    /// `null` when the key was unset.
    pub before: Value,
    /// `null` when the key was removed.
    pub after: Value,
    pub layer: ConfigLayer,
}

pub struct ConfigChanged;
impl EventType for ConfigChanged {
    const TYPE: &'static str = "config.changed";
    const V: u32 = 2;
    type Payload = ConfigChangedV2;
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

/// `capability.switched@1`: a capability's active implementation changed
/// — a person's or the project's choice, or the chosen one coming or
/// going (`.context/work-tracking.md` "Capabilities").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySwitchedV1 {
    /// `work_items`, `effort_policy`, `snapshots`.
    pub capability: String,
    /// The implementation id that was active (`none` for nothing).
    pub from: String,
    /// The one active now.
    pub to: String,
    /// Why it's the one now.
    pub chosen_by: crate::capability::ChosenBy,
}

pub struct CapabilitySwitched;
impl EventType for CapabilitySwitched {
    const TYPE: &'static str = "capability.switched";
    const V: u32 = 1;
    type Payload = CapabilitySwitchedV1;
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

/// `snapshot.taken@3` (tsk883): v1, its trigger able to say
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

/// `vcs.head.moved@2`: HEAD moved while the worktree was clean, so the
/// latest snapshot now also is that commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VcsHeadMovedV2 {
    /// The stream (`str1`).
    pub stream: String,
    /// The snapshot re-stamped with the new commit.
    pub snapshot: i64,
    /// The commit (its sha) the snapshot pointed at before, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The commit (its sha) HEAD points at now.
    pub to: String,
}

pub struct VcsHeadMoved;
impl EventType for VcsHeadMoved {
    const TYPE: &'static str = "vcs.head.moved";
    const V: u32 = 2;
    type Payload = VcsHeadMovedV2;
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

/// `agent.turn.started@2`: a person's prompt opened a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnStartedV2 {
    /// The turn (`trn12`).
    pub turn: String,
    /// The thread (`thr3`).
    pub thread: String,
    /// The harness session id, when it reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

pub struct AgentTurnStarted;
impl EventType for AgentTurnStarted {
    const TYPE: &'static str = "agent.turn.started";
    const V: u32 = 2;
    type Payload = AgentTurnStartedV2;
}

/// A turn's token counts, when the harness reported them with the turn
/// (ACP's prompt response). Claude's come from the transcript instead.
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

/// `agent.turn.ended@3`: a turn closed, with what the token-usage reactor
/// needs to count it — the transcript to read, or the counts themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnEndedV3 {
    /// The turn (`trn12`).
    pub turn: String,
    /// The thread (`thr3`).
    pub thread: String,
    pub outcome: crate::hook::TurnOutcome,
    /// The harness's session transcript (Claude's `transcript_path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// Counts reported with the turn (ACP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
}

pub struct AgentTurnEnded;
impl EventType for AgentTurnEnded {
    const TYPE: &'static str = "agent.turn.ended";
    const V: u32 = 3;
    type Payload = AgentTurnEndedV3;
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
    /// prompt, a final message ending in a question.
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
/// fell in — exports lag the turn — and the thread's single open effort.
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

/// `agent.prompt.submitted@2`: a person sent the agent a prompt. Every
/// prompt logs one; only a prompt with no turn open also opens a turn
/// (`reprompt` says it landed inside an open one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPromptSubmittedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn it opened (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub reprompt: bool,
    /// The text the person submitted, stored in `event_content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<ContentRef>,
}

pub struct AgentPromptSubmitted;
impl EventType for AgentPromptSubmitted {
    const TYPE: &'static str = "agent.prompt.submitted";
    const V: u32 = 2;
    type Payload = AgentPromptSubmittedV2;
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
pub struct AgentToolRequestedV3 {
    pub tool: String,
    pub kind: crate::agent::tool::ToolKind,
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
    const V: u32 = 3;
    type Payload = AgentToolRequestedV3;
}

/// `agent.tool.finished@3`: a tool call returned (PostToolUse). As
/// `agent.tool.finished@2`, plus the subagent that made the call, when it
/// wasn't the session's own agent. v2 upcasts as is (no subagent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentToolFinishedV3 {
    pub tool: String,
    pub kind: crate::agent::tool::ToolKind,
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
    const V: u32 = 3;
    type Payload = AgentToolFinishedV3;
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

/// `agent.peer_message.received@2`: another agent session's message
/// reached the thread's agent as a prompt (Claude Code's cross-session
/// message). Observed only: oxplow never sends one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPeerMessageReceivedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn it opened, or the one it arrived in (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// The sender's own name for its session (its socket when it has no
    /// name).
    pub from: String,
}

pub struct AgentPeerMessageReceived;
impl EventType for AgentPeerMessageReceived {
    const TYPE: &'static str = "agent.peer_message.received";
    const V: u32 = 2;
    type Payload = AgentPeerMessageReceivedV2;
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

/// `agent.plan.proposed@2`: the agent put a plan to the person before
/// acting (Claude's `ExitPlanMode`, Codex's `plan` item; opencode's
/// `plan_exit` carries no text, so it records none). Whether the person
/// accepted it is in no hook, so it isn't recorded. The text sits in the
/// payload, capped, so `v_agent_plan` reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanProposedV2 {
    /// The thread (`thr3`).
    pub thread: String,
    /// The turn (`trn12`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    pub text: String,
    /// The file the harness saved it in (Claude's `planFilePath`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub struct AgentPlanProposed;
impl EventType for AgentPlanProposed {
    const TYPE: &'static str = "agent.plan.proposed";
    const V: u32 = 2;
    type Payload = AgentPlanProposedV2;
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

/// `effort.opened@3`: a span of a thread's work began, linked to a work
/// item or not (`.context/work-tracking.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortOpenedV3 {
    /// The effort (`eff12`).
    pub effort: String,
    /// `oxplow:oxplow/work_item?id=tsk42&provider=oxplow`, or another provider's item, when linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The thread doing the work (`thr3`).
    pub thread: String,
    /// The start snapshot, when the open already had it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_snapshot: Option<i64>,
}

pub struct EffortOpened;
impl EventType for EffortOpened {
    const TYPE: &'static str = "effort.opened";
    const V: u32 = 3;
    type Payload = EffortOpenedV3;
}

/// `effort.claim_verified@2` (P7.C4): a reviewer verified one of an
/// effort's claims (`oxplow.effort.verify_claim`), naming what backs it — or
/// took that back (`oxplow.effort.unverify_claim`, `evidence` absent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortClaimVerifiedV2 {
    /// The claim's id.
    pub claim: i64,
    /// The effort it's on (`eff12`), when on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What backs it now (`reviewer`, `oxplow:oxplow/run?id=12`); absent once unverified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

pub struct EffortClaimVerified;
impl EventType for EffortClaimVerified {
    const TYPE: &'static str = "effort.claim_verified";
    const V: u32 = 2;
    type Payload = EffortClaimVerifiedV2;
}

/// `effort.claim_dismissed@2`: a reviewer dismissed one of an effort's
/// claims (`oxplow.effort.dismiss_claim`: the agent didn't claim it, or
/// didn't mean it) — or took that back (`oxplow.effort.reopen_claim`,
/// `dismissed: false`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortClaimDismissedV2 {
    /// The claim's id.
    pub claim: i64,
    /// The effort it's on (`eff12`), when on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Dismissed now; `false` once reopened.
    pub dismissed: bool,
}

pub struct EffortClaimDismissed;
impl EventType for EffortClaimDismissed {
    const TYPE: &'static str = "effort.claim_dismissed";
    const V: u32 = 2;
    type Payload = EffortClaimDismissedV2;
}

/// `effort.decision_reviewed@2` (P7.C4): a reviewer confirmed or
/// dismissed an inferred decision, or reopened one they had reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortDecisionReviewedV2 {
    /// The decision's id.
    pub decision: i64,
    /// The effort it's on (`eff12`), when on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `confirmed`, `dismissed` or `inferred` (reopened).
    pub outcome: String,
}

pub struct EffortDecisionReviewed;
impl EventType for EffortDecisionReviewed {
    const TYPE: &'static str = "effort.decision_reviewed";
    const V: u32 = 2;
    type Payload = EffortDecisionReviewedV2;
}

/// `effort.decisions_inferred@2`: a model's proposals replaced an
/// effort's inferred decisions (`oxplow.effort.replace_inferred_decisions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortDecisionsInferredV2 {
    /// The effort (`eff12`).
    pub effort: String,
    /// The effort's work item, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// How many inferred decisions it holds now.
    pub count: u32,
}

pub struct EffortDecisionsInferred;
impl EventType for EffortDecisionsInferred {
    const TYPE: &'static str = "effort.decisions_inferred";
    const V: u32 = 2;
    type Payload = EffortDecisionsInferredV2;
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
/// what changed since the turn began — what an effort policy reacts to
/// without reading snapshots or harness tool names
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

/// `work_item.state_changed@1`: a `work_item.*` command put an item in
/// a canonical state — logged by core for every provider, anchored to the
/// thread of whoever ran it (`.context/work-items.md`). A create always
/// logs one; for a provider oxplow can't read the prior state of, a
/// transition or update that names a state logs one even when the item
/// was already there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemStateChangedV1 {
    /// `oxplow:oxplow/work_item?id=<id>&provider=<provider>`.
    pub work_item: String,
    pub to: crate::work_items::CanonicalState,
}

pub struct WorkItemStateChanged;
impl EventType for WorkItemStateChanged {
    const TYPE: &'static str = "work_item.state_changed";
    const V: u32 = 1;
    type Payload = WorkItemStateChangedV1;
}

/// `effort.linked@2`: an effort's work item was set, changed or cleared
/// (`oxplow.effort.link`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortLinkedV2 {
    /// The effort (`eff12`).
    pub effort: String,
    /// The work item it's linked to now; absent when unlinked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
}

pub struct EffortLinked;
impl EventType for EffortLinked {
    const TYPE: &'static str = "effort.linked";
    const V: u32 = 2;
    type Payload = EffortLinkedV2;
}

/// `effort.landed@2`: a commit holds an open effort's work — every file
/// the effort changed is in the commit as the effort left it (`complete`),
/// or only some of them. What an effort policy closes an effort on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortLandedV2 {
    /// The effort (`eff12`).
    pub effort: String,
    /// The commit's sha.
    pub commit: String,
    pub complete: bool,
}

pub struct EffortLanded;
impl EventType for EffortLanded {
    const TYPE: &'static str = "effort.landed";
    const V: u32 = 2;
    type Payload = EffortLandedV2;
}

/// `effort.retitled@2`: an effort's own title was set or cleared
/// (`oxplow.effort.update`); cleared, it shows its default again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortRetitledV2 {
    /// The effort (`eff12`).
    pub effort: String,
    /// Its title now; absent when cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

pub struct EffortRetitled;
impl EventType for EffortRetitled {
    const TYPE: &'static str = "effort.retitled";
    const V: u32 = 2;
    type Payload = EffortRetitledV2;
}

/// `effort.closed@3`: a span of a thread's work ended, linked to a work
/// item or not, and what closed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortClosedV3 {
    /// The effort (`eff12`).
    pub effort: String,
    /// The linked work item, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The end snapshot, when the close already had it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_snapshot: Option<i64>,
    /// What closed it: `commit`, `switch`, `person`, `agent` or `system`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<String>,
}

pub struct EffortClosed;
impl EventType for EffortClosed {
    const TYPE: &'static str = "effort.closed";
    const V: u32 = 3;
    type Payload = EffortClosedV3;
}

// The work-item events are the interface's: core logs them for every
// list, from the verb a `oxplow.work_item.*` command ran and what the list
// answered (`.context/work-items.md`).

/// `work_item.created@2`: an item was filed on a work list, in `state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCreatedV2 {
    /// `oxplow:oxplow/work_item?id=<id>&provider=<list>`.
    pub work_item: String,
    /// The canonical state it was filed in.
    pub state: crate::work_items::CanonicalState,
}

pub struct WorkItemCreated;
impl EventType for WorkItemCreated {
    const TYPE: &'static str = "work_item.created";
    const V: u32 = 2;
    type Payload = WorkItemCreatedV2;
}

/// `work_item.deleted@2`: an item was deleted from its work list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemDeletedV2 {
    /// `oxplow:oxplow/work_item?id=<id>&provider=<list>`.
    pub work_item: String,
}

pub struct WorkItemDeleted;
impl EventType for WorkItemDeleted {
    const TYPE: &'static str = "work_item.deleted";
    const V: u32 = 2;
    type Payload = WorkItemDeletedV2;
}

/// `work_item.edited@2`: a command changed an item's fields — not its
/// state, which is `work_item.state_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemEditedV2 {
    /// `oxplow:oxplow/work_item?id=<id>&provider=<list>`.
    pub work_item: String,
    /// The fields the command set: `title`, `body`, `parent`, `rank` (its
    /// place in its list), `list` (moved to another), and
    /// `native.<name>` for each of the list's own fields.
    pub fields: Vec<String>,
}

pub struct WorkItemEdited;
impl EventType for WorkItemEdited {
    const TYPE: &'static str = "work_item.edited";
    const V: u32 = 2;
    type Payload = WorkItemEditedV2;
}

/// `work_item.linked@2`: a typed link from an item to another of the
/// same list, or to any ref it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemLinkedV2 {
    /// The item linked from (`oxplow:oxplow/work_item?id=<id>&provider=<list>`).
    pub work_item: String,
    /// What it links to: an item, or any ref.
    pub target: String,
    /// One of the list's own link types.
    pub link_type: String,
}

pub struct WorkItemLinked;
impl EventType for WorkItemLinked {
    const TYPE: &'static str = "work_item.linked";
    const V: u32 = 2;
    type Payload = WorkItemLinkedV2;
}

/// `work_item.unlinked@1`: an item's link was removed
/// (`oxplow.work_item.unlink`, or undoing a link).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemUnlinkedV1 {
    /// The item linked from (`oxplow:oxplow/work_item?id=<id>&provider=<list>`).
    pub work_item: String,
    /// What it linked to.
    pub target: String,
    pub link_type: String,
}

pub struct WorkItemUnlinked;
impl EventType for WorkItemUnlinked {
    const TYPE: &'static str = "work_item.unlinked";
    const V: u32 = 1;
    type Payload = WorkItemUnlinkedV1;
}

/// `work_item.commented@2`: a comment was added to an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCommentedV2 {
    /// `oxplow:oxplow/work_item?id=<id>&provider=<list>`.
    pub work_item: String,
    /// The list's own id for the comment, unique on the item, when its
    /// answer named one (the `comment` verb's `result.comment`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

pub struct WorkItemCommented;
impl EventType for WorkItemCommented {
    const TYPE: &'static str = "work_item.commented";
    const V: u32 = 2;
    type Payload = WorkItemCommentedV2;
}

/// `work_item.recorded@2`: a list's item as it now stands — how every
/// list's items reach the work-item interface (the `work_items.project`
/// consumer upserts it by ref), with its rank, links, comments and list
/// when it states them. A list's verbs answer with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemRecordedV2 {
    pub item: crate::work_items::WorkItemRecord,
}

pub struct WorkItemRecorded;
impl EventType for WorkItemRecorded {
    const TYPE: &'static str = "work_item.recorded";
    const V: u32 = 2;
    type Payload = WorkItemRecordedV2;
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

/// `knowledge.page.written@2`: a knowledge page's row and edges were
/// restated from its body — by `oxplow.knowledge.write_page` / `link` /
/// `resync`, or by the wiki watcher after a hand edit
/// (`system:wiki_watch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageWrittenV2 {
    /// The page's slug.
    pub page: String,
    /// Every ref the page now points at (`oxplow:oxplow/file?path=src/lib.rs`, `oxplow:oxplow/wiki?slug=x`, …).
    pub outbound: Vec<String>,
    /// The snapshot its new refs are pinned to, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<i64>,
}

pub struct KnowledgePageWritten;
impl EventType for KnowledgePageWritten {
    const TYPE: &'static str = "knowledge.page.written";
    const V: u32 = 2;
    type Payload = KnowledgePageWrittenV2;
}

/// A knowledge page as a store records it: what a provider's write verb
/// answers and its collector streams. The body is what it keeps; the refs
/// are what the page links, in oxplow's grammar (`oxplow:oxplow/file?path=src/lib.rs`,
/// `oxplow:oxplow/wiki?slug=other`), which core pins and links from — never parsed out of a
/// body in the store's own grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageRecord {
    /// `oxplow:oxplow/wiki?slug=<slug>`.
    #[serde(rename = "ref")]
    pub page_ref: String,
    pub title: String,
    pub body: String,
    pub refs: Vec<String>,
    /// RFC 3339: when the store last changed it.
    pub updated_at: String,
    /// It was deleted in the store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

/// `knowledge.page.recorded@1`: a knowledge store's page, as it recorded
/// it (`.context/knowledge.md` "The capability").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageRecordedV1 {
    pub page: KnowledgePageRecord,
}

pub struct KnowledgePageRecorded;
impl EventType for KnowledgePageRecorded {
    const TYPE: &'static str = "knowledge.page.recorded";
    const V: u32 = 1;
    type Payload = KnowledgePageRecordedV1;
}

/// `lens.shown@2`: an agent showed the person an answer in a thread —
/// an existing lens or its own lens spec (`oxplow.lens.show`, P6.C1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensShownV2 {
    /// The answer's id.
    pub answer: i64,
    /// The thread (`thr3`).
    pub thread: String,
    /// The lens (`<extension>/<slug>`) when it shows an existing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
}

pub struct LensShown;
impl EventType for LensShown {
    const TYPE: &'static str = "lens.shown";
    const V: u32 = 2;
    type Payload = LensShownV2;
}

/// `lens.kept@3` (P11, tsk943): a lens was kept — an answer from a thread,
/// or a spec (Explore Data's Save as Lens) — written as a private lens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensKeptV3 {
    /// The answer's id, when it was an answer that was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<i64>,
    /// The lens (`<extension>/<slug>`).
    pub lens: String,
}

pub struct LensKept;
impl EventType for LensKept {
    const TYPE: &'static str = "lens.kept";
    const V: u32 = 3;
    type Payload = LensKeptV3;
}

/// `ui.op_failed@1` (tsk1072): an operation the person started in the
/// app failed, as the app showed it (`oxplow.ui.report_error`). Its captured
/// stderr and stdout are the `output` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOpFailedV1 {
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
}

pub struct UiOpFailed;
impl EventType for UiOpFailed {
    const TYPE: &'static str = "ui.op_failed";
    const V: u32 = 1;
    type Payload = UiOpFailedV1;
}

/// `knowledge.page.deleted@2`: a knowledge page is gone, its row and
/// edges with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageDeletedV2 {
    /// The page's slug.
    pub page: String,
}

pub struct KnowledgePageDeleted;
impl EventType for KnowledgePageDeleted {
    const TYPE: &'static str = "knowledge.page.deleted";
    const V: u32 = 2;
    type Payload = KnowledgePageDeletedV2;
}

/// `knowledge.note.written@3`: a thread note was added or its body
/// changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeNoteWrittenV3 {
    /// The note (`not3`).
    pub note: String,
    /// The thread it's on (`thr3`).
    pub thread: String,
}

pub struct KnowledgeNoteWritten;
impl EventType for KnowledgeNoteWritten {
    const TYPE: &'static str = "knowledge.note.written";
    const V: u32 = 3;
    type Payload = KnowledgeNoteWrittenV3;
}

/// `knowledge.note.deleted@3`: a thread note is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeNoteDeletedV3 {
    /// The note (`not3`).
    pub note: String,
    /// The thread it was on (`thr3`).
    pub thread: String,
}

pub struct KnowledgeNoteDeleted;
impl EventType for KnowledgeNoteDeleted {
    const TYPE: &'static str = "knowledge.note.deleted";
    const V: u32 = 3;
    type Payload = KnowledgeNoteDeletedV3;
}

/// `handoff.sent@2`: work was handed from one thread to another
/// (`oxplow.handoff.send`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffSentV2 {
    /// The handoff (`hnd1`).
    pub handoff: String,
    /// The sending thread (`thr3`).
    pub from_thread: String,
    /// The receiving thread (`thr4`).
    pub to_thread: String,
    /// The work item it hands over, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
}

pub struct HandoffSent;
impl EventType for HandoffSent {
    const TYPE: &'static str = "handoff.sent";
    const V: u32 = 2;
    type Payload = HandoffSentV2;
}

/// `handoff.settled@2`: a handoff's state moved (`oxplow.handoff.settle`):
/// taken or declined by its recipient, withdrawn by its sender, or open
/// again (an undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffSettledV2 {
    /// The handoff (`hnd1`).
    pub handoff: String,
    /// `open`, `taken`, `declined` or `withdrawn`.
    pub state: String,
    /// The sending thread (`thr3`).
    pub from_thread: String,
    /// The receiving thread (`thr4`).
    pub to_thread: String,
}

pub struct HandoffSettled;
impl EventType for HandoffSettled {
    const TYPE: &'static str = "handoff.settled";
    const V: u32 = 2;
    type Payload = HandoffSettledV2;
}

/// `knowledge.comment.written@3`: a comment (a threaded annotation on a
/// page) was created or changed — a reply, its intent, status or anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCommentWrittenV3 {
    /// The comment (`cmt5`).
    pub comment: String,
    /// What it annotates: a URI.
    pub target: String,
}

pub struct KnowledgeCommentWritten;
impl EventType for KnowledgeCommentWritten {
    const TYPE: &'static str = "knowledge.comment.written";
    const V: u32 = 3;
    type Payload = KnowledgeCommentWrittenV3;
}

/// `knowledge.comment.deleted@3`: a comment is gone, its messages with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCommentDeletedV3 {
    /// The comment (`cmt5`).
    pub comment: String,
    /// What it annotated: a URI.
    pub target: String,
}

pub struct KnowledgeCommentDeleted;
impl EventType for KnowledgeCommentDeleted {
    const TYPE: &'static str = "knowledge.comment.deleted";
    const V: u32 = 3;
    type Payload = KnowledgeCommentDeletedV3;
}

/// `effort.finished@3`: a closed effort has its end snapshot
/// pinned and its lifecycle metrics projected. Logged once per effort by the
/// effort-lifecycle consumer (dedupe key `effort.finished:<effort>`),
/// caused by the `effort.closed` it handled; what the effort reactors
/// (evidence, inferred decisions) and the `effort.finished` collectors
/// consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortFinishedV3 {
    /// The effort (`eff12`).
    pub effort: String,
    /// The linked work item, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item: Option<String>,
    /// The end snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_snapshot: Option<i64>,
}

pub struct EffortFinished;
impl EventType for EffortFinished {
    const TYPE: &'static str = "effort.finished";
    const V: u32 = 3;
    type Payload = EffortFinishedV3;
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
                ("agent.peer_message.received", 2),
                ("agent.plan.proposed", 2),
                ("agent.prompt.submitted", 2),
                ("agent.session.ended", 2),
                ("agent.session.started", 3),
                ("agent.status.changed", 2),
                ("agent.subagent.finished", 2),
                ("agent.subagent.started", 2),
                ("agent.tokens.reported", 2),
                ("agent.tool.finished", 3),
                ("agent.tool.requested", 3),
                ("agent.turn.ended", 3),
                ("agent.turn.started", 2),
                ("capability.switched", 1),
                ("code.diagnostics.changed", 2),
                ("collector.synced", 2),
                ("command.approved", 2),
                ("command.declined", 2),
                ("command.executed", 4),
                ("command.proposed", 3),
                ("config.changed", 2),
                ("contribution.disabled", 1),
                ("contribution.enabled", 1),
                ("contribution.job.completed", 2),
                ("effect.result", 5),
                ("effort.claim_dismissed", 2),
                ("effort.claim_verified", 2),
                ("effort.closed", 3),
                ("effort.decision_reviewed", 2),
                ("effort.decisions_inferred", 2),
                ("effort.finished", 3),
                ("effort.landed", 2),
                ("effort.linked", 2),
                ("effort.opened", 3),
                ("effort.retitled", 2),
                ("file.created", 1),
                ("file.deleted", 1),
                ("file.directory_created", 1),
                ("file.renamed", 1),
                ("file.saved", 1),
                ("handoff.sent", 2),
                ("handoff.settled", 2),
                ("knowledge.comment.deleted", 3),
                ("knowledge.comment.written", 3),
                ("knowledge.note.deleted", 3),
                ("knowledge.note.written", 3),
                ("knowledge.page.deleted", 2),
                ("knowledge.page.recorded", 1),
                ("knowledge.page.written", 2),
                ("lens.kept", 3),
                ("lens.shown", 2),
                ("snapshot.taken", 3),
                ("test.coverage.recorded", 1),
                ("test.run.recorded", 3),
                ("thread.checkpoint", 2),
                ("ui.op_failed", 1),
                ("vcs.commit.indexed", 2),
                ("vcs.head.moved", 2),
                ("work_item.commented", 2),
                ("work_item.created", 2),
                ("work_item.deleted", 2),
                ("work_item.edited", 2),
                ("work_item.linked", 2),
                ("work_item.recorded", 2),
                ("work_item.state_changed", 1),
                ("work_item.unlinked", 1),
            ]
        );
        assert_eq!(r.latest("work_item.created"), Some(2));
        assert_eq!(r.latest("agent.turn.ended"), Some(3));
        assert_eq!(r.owner("config.changed", 2), Some(None));
    }

    #[test]
    fn validate_accepts_the_typed_shape_and_names_the_violation() {
        let r = crate::vocabulary::Vocabulary::core();
        let ok = Envelope::typed::<WorkItemStateChanged>(
            "human",
            &WorkItemStateChangedV1 {
                work_item: "oxplow:oxplow/work_item?id=tsk4&provider=oxplow".into(),
                to: crate::work_items::CanonicalState::InProgress,
            },
        );
        assert_eq!(ok.event_type, "work_item.state_changed");
        assert_eq!(ok.v, 1);
        r.validate_envelope(&ok).unwrap();

        let bad = r
            .validate(
                "work_item.state_changed",
                1,
                &json!({"work_item": "x", "to": "flying"}),
            )
            .unwrap_err();
        let msg = bad.to_string();
        assert!(
            msg.contains("work_item.state_changed@1") && msg.contains("/to"),
            "{msg}"
        );

        let extra = r
            .validate(
                "config.changed",
                2,
                &json!({"key": "zones", "before": null, "after": 1, "layer": "project", "oops": 1}),
            )
            .unwrap_err();
        assert!(extra.to_string().contains("oops"), "{extra}");
    }

    #[test]
    fn unknown_types_and_versions_are_refused() {
        let r = EventSchemaRegistry::core();
        let e = r
            .validate("work_item.state_changed", 2, &json!({}))
            .unwrap_err();
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
        const TYPE: &'static str = "acme_review.decided";
        const V: u32 = 1;
        type Payload = ConfigChangedV2;
    }
    struct Squatter;
    impl EventType for Squatter {
        const TYPE: &'static str = "work_item.hijacked";
        const V: u32 = 1;
        type Payload = ConfigChangedV2;
    }

    #[test]
    fn an_extension_declares_types_only_under_its_own_namespace() {
        let mut r = EventSchemaRegistry::core();
        // The namespace is the extension's name with `-` read as `_`.
        r.register_declared("acme-review", declared::<AcmeDecided>())
            .unwrap();
        assert_eq!(r.owner("acme_review.decided", 1), Some(Some("acme-review")));
        assert_eq!(
            r.summary("acme_review.decided", 1),
            Some("acme_review.decided, for the test")
        );
        let core = r
            .register_declared("acme-review", declared::<Squatter>())
            .unwrap_err();
        assert!(core.to_string().contains("core namespace"), "{core}");
        let foreign = r
            .register_declared("other-extension", declared::<AcmeDecided>())
            .unwrap_err();
        assert!(
            foreign.to_string().contains("other_extension.*"),
            "{foreign}"
        );
        let dup = r
            .register_declared("acme-review", declared::<AcmeDecided>())
            .unwrap_err();
        assert!(dup.to_string().contains("already registered"), "{dup}");
        // Core can't register into an extension's namespace either, and a
        // second registration of the same type@v collides.
        assert!(r.register::<AcmeDecided>().is_err());
        assert!(r.register::<WorkItemStateChanged>().is_err());
    }

    #[test]
    fn a_declared_type_needs_a_schema_that_compiles_and_an_upcast_past_v1() {
        let mut r = EventSchemaRegistry::new();
        let bad = r
            .register_declared(
                "acme",
                DeclaredEventType {
                    schema: json!({"type": "nonsense"}),
                    ..declared::<Thing1>()
                },
            )
            .unwrap_err();
        assert!(bad.to_string().contains("acme.thing@1"), "{bad}");
        let no_upcast = r
            .register_declared("acme", declared::<Thing2>())
            .unwrap_err();
        assert!(no_upcast.to_string().contains("upcast"), "{no_upcast}");
        let zero = r
            .register_declared(
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
        r.register_declared("acme", declared::<Thing1>()).unwrap();
        r.register_declared(
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
        assert!(WorkItemStateChanged::upcast(0, json!({})).is_err());
    }
}
