//! Event payload schemas: one Rust type per `type@v`, a registry that
//! validates payloads on append, and the golden-schema discipline that
//! keeps published event shapes from drifting.
//!
//! **Golden schemas.** Every registered core type's JSON Schema is
//! checked in at `crates/oxplow-domain/schemas/events/<type>@<v>.json`.
//! A test regenerates each schema from its Rust type and fails if the
//! file differs: a published `type@v` is a contract consumers (plugins,
//! lenses, the agent) were written against, so a change is a **new
//! version** (`V + 1`, with an `upcast` from the old shape), never an
//! edit. Run the test with `OXPLOW_BLESS=1` to write a new golden.
//!
//! **Namespaces.** Core types live in the namespaces of
//! `.context/target-architecture.md` §5.3 ([`CORE_NAMESPACES`]). An
//! extension declares types (a JSON Schema each, not a Rust type —
//! [`DeclaredEventType`]) only under its own namespace
//! ([`plugin_namespace`]); declaring into a core namespace, or under
//! another extension's, is refused.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{validate_type_name, Envelope};
use crate::task::TaskStatus;
use crate::DomainError;

/// The namespaces core owns (§5.3). Plugin types may not use them.
pub const CORE_NAMESPACES: &[&str] = &[
    "agent",
    "snapshot",
    "vcs",
    "effort",
    "work_item",
    "knowledge",
    "test",
    "code",
    "command",
    "effect",
    "collector",
    "lens",
    "config",
    "provider",
    "plugin",
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

/// The namespace an extension's types live under: its name, `-` read as
/// `_` (a type name is snake_case).
pub fn plugin_namespace(extension: &str) -> String {
    extension.replace('-', "_")
}

struct Registered {
    validator: jsonschema::Validator,
    schema: Value,
    /// Who registered it: `None` for core, `Some(extension)` otherwise.
    owner: Option<String>,
    /// What it records; core types say so in their Rust docs instead.
    summary: Option<String>,
}

/// Every event type the log accepts, by `type@v`, with the newest version
/// of each type and its upcast chain.
#[derive(Default)]
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
        r.register::<WorkItemTransitioned>()
            .expect("core type registers");
        r.register::<CommandExecuted>()
            .expect("core type registers");
        r.register::<ConfigChanged>().expect("core type registers");
        r.register::<EffectResult>().expect("core type registers");
        r.register::<SnapshotTaken>().expect("core type registers");
        r.register::<VcsHeadMoved>().expect("core type registers");
        r.register::<AgentTurnStarted>()
            .expect("core type registers");
        r.register::<AgentTurnEndedAtV1>()
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
        r.register::<AgentStatusChanged>()
            .expect("core type registers");
        r.register::<TestRunRecorded>()
            .expect("core type registers");
        r.register::<TestCoverageRecorded>()
            .expect("core type registers");
        r.register::<WorkItemDeleted>()
            .expect("core type registers");
        r.register::<EffortOpened>().expect("core type registers");
        r.register::<EffortClosed>().expect("core type registers");
        r.register::<EffortClaimVerified>()
            .expect("core type registers");
        r.register::<EffortDecisionReviewed>()
            .expect("core type registers");
        r.register::<EffortFinished>().expect("core type registers");
        r.register::<CollectorSynced>()
            .expect("core type registers");
        r.register::<WorkItemEdited>().expect("core type registers");
        r.register::<WorkItemCreated>()
            .expect("core type registers");
        r.register::<WorkItemLinked>().expect("core type registers");
        r.register::<WorkItemCommented>()
            .expect("core type registers");
        r.register::<WorkItemRecorded>()
            .expect("core type registers");
        r.register::<KnowledgePageWritten>()
            .expect("core type registers");
        r.register::<KnowledgePageDeleted>()
            .expect("core type registers");
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
        r.register::<PluginEnabled>().expect("core type registers");
        r.register::<PluginDisabled>().expect("core type registers");
        r.register::<LensShown>().expect("core type registers");
        r.register::<LensKept>().expect("core type registers");
        r.register::<CommandProposed>()
            .expect("core type registers");
        r.register::<CommandApproved>()
            .expect("core type registers");
        r.register::<CommandDeclined>()
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
        let own = plugin_namespace(extension);
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
                validator,
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
    /// for a plugin, `None` when unregistered.
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

/// `work_item.transitioned@1`: a task changed status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemTransitionedV1 {
    /// The task's canonical ref (`work_item:oxplow:tsk42`).
    pub work_item: String,
    pub from: TaskStatus,
    pub to: TaskStatus,
    /// The effort this transition opened or closed, when it did either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

pub struct WorkItemTransitioned;
impl EventType for WorkItemTransitioned {
    const TYPE: &'static str = "work_item.transitioned";
    const V: u32 = 1;
    type Payload = WorkItemTransitionedV1;
}

/// Who ran a command (`.context/target-architecture.md` §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    Lens,
    System,
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

/// `command.executed@1`: the bus ran a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandExecutedV1 {
    /// The command's name (`work_item.transition`, `config.set`).
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
    const V: u32 = 1;
    type Payload = CommandExecutedV1;
}

/// `command.proposed@1`: an agent ran a command that needs a person's
/// confirmation; it is kept as a proposal until a person decides (P6b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandProposedV1 {
    /// `proposal:<id>`.
    pub proposal: String,
    /// The command's name (`config.set`).
    pub command: String,
    pub actor_kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Whether the command is destructive.
    pub destructive: bool,
    /// The pending proposals of the same call (`proposal:<id>`) this one
    /// replaced — marked superseded in the same transaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
}

pub struct CommandProposed;
impl EventType for CommandProposed {
    const TYPE: &'static str = "command.proposed";
    const V: u32 = 1;
    type Payload = CommandProposedV1;
}

/// `command.approved@1`: a person approved a proposal and the command ran
/// as them, audited as `audit_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandApprovedV1 {
    /// `proposal:<id>`.
    pub proposal: String,
    pub command: String,
    /// The approving run's `command_audit` row.
    pub audit_id: i64,
}

pub struct CommandApproved;
impl EventType for CommandApproved {
    const TYPE: &'static str = "command.approved";
    const V: u32 = 1;
    type Payload = CommandApprovedV1;
}

/// `command.declined@1`: a person declined a proposal; nothing ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandDeclinedV1 {
    /// `proposal:<id>`.
    pub proposal: String,
    pub command: String,
}

pub struct CommandDeclined;
impl EventType for CommandDeclined {
    const TYPE: &'static str = "command.declined";
    const V: u32 = 1;
    type Payload = CommandDeclinedV1;
}

/// `config.changed@1`: one project config key changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigChangedV1 {
    /// The `ConfigKey` (`zones`, `metricRetentionDays`).
    pub key: String,
    /// `null` when the key was unset.
    pub before: Value,
    /// `null` when the key was removed.
    pub after: Value,
}

pub struct ConfigChanged;
impl EventType for ConfigChanged {
    const TYPE: &'static str = "config.changed";
    const V: u32 = 1;
    type Payload = ConfigChangedV1;
}

/// `effect.result@1`: the recorded outcome of a side effect (§5.3).
/// Shape only in P1; the effects runtime that emits it is later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectResultV1 {
    /// The effect's name (`notify.desktop`, `git.push`).
    pub effect: String,
    /// The canonical ref the effect acted on, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Effect-specific detail, opaque to the log.
    #[serde(default)]
    pub detail: Value,
}

pub struct EffectResult;
impl EventType for EffectResult {
    const TYPE: &'static str = "effect.result";
    const V: u32 = 1;
    type Payload = EffectResultV1;
}

/// `snapshot.taken@1`: one snapshot take (one `snapshot_op` row). Refs
/// are canonical (`.context/refs.md`): `stream:str1`, `snapshot:123`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotTakenV1 {
    pub stream: String,
    /// The snapshot the worktree is at after the take: a new one, or the
    /// parent when nothing changed (`unchanged`).
    pub snapshot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
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
    const V: u32 = 1;
    type Payload = SnapshotTakenV1;
}

/// `vcs.head.moved@1`: HEAD moved while the worktree was clean, so the
/// latest snapshot now also is that commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VcsHeadMovedV1 {
    pub stream: String,
    /// The snapshot re-stamped with the new commit.
    pub snapshot: String,
    /// `commit:<sha>` the snapshot pointed at before, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `commit:<sha>` HEAD points at now.
    pub to: String,
}

pub struct VcsHeadMoved;
impl EventType for VcsHeadMoved {
    const TYPE: &'static str = "vcs.head.moved";
    const V: u32 = 1;
    type Payload = VcsHeadMovedV1;
}

/// `agent.turn.started@1`: a person's prompt opened a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnStartedV1 {
    /// `turn:trn12`.
    pub turn: String,
    /// `thread:thr3`.
    pub thread: String,
    /// The harness session id, when it reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

pub struct AgentTurnStarted;
impl EventType for AgentTurnStarted {
    const TYPE: &'static str = "agent.turn.started";
    const V: u32 = 1;
    type Payload = AgentTurnStartedV1;
}

// Superseded by v2; kept registered so rows logged at v1 still validate
// and upcast. (Its doc comment is part of the published schema.)
/// `agent.turn.ended@1`: a turn closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnEndedV1 {
    pub turn: String,
    pub thread: String,
    pub outcome: crate::hook::TurnOutcome,
}

/// The v1 shape of `agent.turn.ended`, as a registry entry.
pub struct AgentTurnEndedAtV1;
impl EventType for AgentTurnEndedAtV1 {
    const TYPE: &'static str = "agent.turn.ended";
    const V: u32 = 1;
    type Payload = AgentTurnEndedV1;
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

/// `agent.turn.ended@2`: a turn closed, with what the token-usage reactor
/// needs to count it — the transcript to read, or the counts themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnEndedV2 {
    pub turn: String,
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
    const V: u32 = 2;
    type Payload = AgentTurnEndedV2;

    /// v1 → v2 adds two optional fields: a v1 payload is a valid v2 one.
    fn upcast(from_v: u32, payload: Value) -> Result<Value, DomainError> {
        match from_v {
            1 => Ok(payload),
            _ => Err(DomainError::Invalid(format!(
                "agent.turn.ended@{from_v} cannot be upcast to v2"
            ))),
        }
    }
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

/// The agent harness a session ran in — the event's own vocabulary, so
/// the published contract doesn't change with the internal `AgentKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    Claude,
    Codex,
    Opencode,
    /// Any agent spoken to over the Agent Client Protocol.
    Acp,
}

impl From<crate::agent::AgentKind> for Harness {
    fn from(kind: crate::agent::AgentKind) -> Self {
        use crate::agent::AgentKind as K;
        match kind {
            K::Claude => Harness::Claude,
            K::Codex => Harness::Codex,
            K::Opencode => Harness::Opencode,
            K::Acp => Harness::Acp,
        }
    }
}

/// A status a thread's agent can be logged in. There is no `stalled`: that
/// is derived from silence and never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoggedAgentStatus {
    Idle,
    Running,
    /// Parked on the person (`await_user`, a question).
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

/// `agent.session.started@1`: a harness session began on a thread — first
/// seen by id (hooks) or announced (ACP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionStartedV1 {
    pub session: String,
    pub thread: String,
    pub harness: Harness,
    /// The session is the thread's resume session (a reattach), not new.
    pub resumed: bool,
}

pub struct AgentSessionStarted;
impl EventType for AgentSessionStarted {
    const TYPE: &'static str = "agent.session.started";
    const V: u32 = 1;
    type Payload = AgentSessionStartedV1;
}

/// `agent.session.ended@1`: the harness ended a session (`/clear`, exit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionEndedV1 {
    pub session: String,
    pub thread: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub struct AgentSessionEnded;
impl EventType for AgentSessionEnded {
    const TYPE: &'static str = "agent.session.ended";
    const V: u32 = 1;
    type Payload = AgentSessionEndedV1;
}

/// `agent.prompt.submitted@1`: a person sent the agent a prompt. Every
/// prompt logs one; only a prompt with no turn open also opens a turn
/// (`reprompt` says it landed inside an open one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPromptSubmittedV1 {
    pub thread: String,
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
    const V: u32 = 1;
    type Payload = AgentPromptSubmittedV1;
}

/// What oxplow's agent policy said about a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    Allowed,
    Denied,
}

/// `agent.tool.requested@1`: the agent asked to run a tool (PreToolUse),
/// and whether the policy let it. Tool names are the canonical
/// (Claude-shaped) vocabulary every transport maps onto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentToolRequestedV1 {
    pub tool: String,
    /// Repo-relative path the tool targets, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A short summary (a truncated command, a search pattern).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ContentRef>,
    pub decision: ToolDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub struct AgentToolRequested;
impl EventType for AgentToolRequested {
    const TYPE: &'static str = "agent.tool.requested";
    const V: u32 = 1;
    type Payload = AgentToolRequestedV1;
}

/// `agent.tool.finished@1`: a tool call returned (PostToolUse). The
/// recorders — tool-call rows, effort claims, collection — react to this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentToolFinishedV1 {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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
}

pub struct AgentToolFinished;
impl EventType for AgentToolFinished {
    const TYPE: &'static str = "agent.tool.finished";
    const V: u32 = 1;
    type Payload = AgentToolFinishedV1;
}

/// `agent.status.changed@1`: a thread's agent moved to another state.
/// Logged on a transition only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusChangedV1 {
    pub thread: String,
    pub state: LoggedAgentStatus,
    /// The `await_user` question, or why it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub struct AgentStatusChanged;
impl EventType for AgentStatusChanged {
    const TYPE: &'static str = "agent.status.changed";
    const V: u32 = 1;
    type Payload = AgentStatusChangedV1;
}

/// `test.run.recorded@1`: a test run was captured (`run:<capture>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRunRecordedV1 {
    pub run: String,
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
    /// Who produced it: `post-tool-bash`, `plugin-exec:<name>`, `mcp`.
    pub source: String,
}

pub struct TestRunRecorded;
impl EventType for TestRunRecorded {
    const TYPE: &'static str = "test.run.recorded";
    const V: u32 = 1;
    type Payload = TestRunRecordedV1;
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

/// `effort.opened@1`: a bracket of work on a work item began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortOpenedV1 {
    /// `effort:eff12`.
    pub effort: String,
    /// `work_item:oxplow:tsk42`, or another provider's item.
    pub work_item: String,
    /// `thread:thr3` — the thread doing the work.
    pub thread: String,
    /// `snapshot:N`, when the open already had its start snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_snapshot: Option<String>,
    /// Recorded after the fact (attribution for work on an item that was
    /// never opened), so there is no bracket to snapshot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retroactive: bool,
}

pub struct EffortOpened;
impl EventType for EffortOpened {
    const TYPE: &'static str = "effort.opened";
    const V: u32 = 1;
    type Payload = EffortOpenedV1;
}

/// `effort.closed@1`: a bracket of work ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortClosedV1 {
    pub effort: String,
    pub work_item: String,
    /// `snapshot:N`, when the close already had its end snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_snapshot: Option<String>,
    /// Recorded after the fact (attribution for work on an item that was
    /// never opened), so there is no bracket to snapshot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retroactive: bool,
}

/// `effort.claim_verified@1` (P7.C4): a reviewer verified one of an
/// effort's claims (`effort.verify_claim`), naming what backs it — or
/// took that back (`effort.unverify_claim`, `evidence` absent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortClaimVerifiedV1 {
    /// `claim:<id>`.
    pub claim: String,
    /// `effort:<id>`, when the claim was made in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What backs it now (`reviewer`, `run:12`); absent once unverified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

pub struct EffortClaimVerified;
impl EventType for EffortClaimVerified {
    const TYPE: &'static str = "effort.claim_verified";
    const V: u32 = 1;
    type Payload = EffortClaimVerifiedV1;
}

/// `effort.decision_reviewed@1` (P7.C4): a reviewer confirmed or
/// dismissed an inferred decision, or reopened one they had reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortDecisionReviewedV1 {
    /// `decision:<id>`.
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `confirmed`, `dismissed` or `inferred` (reopened).
    pub outcome: String,
}

pub struct EffortDecisionReviewed;
impl EventType for EffortDecisionReviewed {
    const TYPE: &'static str = "effort.decision_reviewed";
    const V: u32 = 1;
    type Payload = EffortDecisionReviewedV1;
}

/// `collector.synced@1` (P7.B3): a collector ran — by hand
/// (`collector.sync`), on its schedule, or for an event its `on:` trigger
/// names (then the envelope's `cause` is that event). Logged in the
/// transaction that wrote what it collected; a run that failed logs one
/// too, with what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectorSyncedV1 {
    /// `collector:<owner>/<id>`.
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
    const V: u32 = 1;
    type Payload = CollectorSyncedV1;
}

pub struct EffortClosed;
impl EventType for EffortClosed {
    const TYPE: &'static str = "effort.closed";
    const V: u32 = 1;
    type Payload = EffortClosedV1;
}

/// `work_item.created@1`: a task was filed, in `status` (filing straight
/// into `in_progress` opens its effort in the same transaction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCreatedV1 {
    /// `work_item:oxplow:tsk42`.
    pub work_item: String,
    pub status: TaskStatus,
    /// The effort filing it opened, when it was filed `in_progress`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

pub struct WorkItemCreated;
impl EventType for WorkItemCreated {
    const TYPE: &'static str = "work_item.created";
    const V: u32 = 1;
    type Payload = WorkItemCreatedV1;
}

/// `work_item.deleted@1`: a task was deleted (soft: its row stays, hidden).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemDeletedV1 {
    pub work_item: String,
}

pub struct WorkItemDeleted;
impl EventType for WorkItemDeleted {
    const TYPE: &'static str = "work_item.deleted";
    const V: u32 = 1;
    type Payload = WorkItemDeletedV1;
}

/// `work_item.edited@1`: a task's own fields changed (not its status —
/// that is `work_item.transitioned`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemEditedV1 {
    /// `work_item:oxplow:tsk42`.
    pub work_item: String,
    /// What changed: `title`, `description`, `priority`, `parent`.
    pub fields: Vec<String>,
}

pub struct WorkItemEdited;
impl EventType for WorkItemEdited {
    const TYPE: &'static str = "work_item.edited";
    const V: u32 = 1;
    type Payload = WorkItemEditedV1;
}

/// `work_item.linked@1`: a typed link from one work item to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemLinkedV1 {
    /// The item linked from.
    pub work_item: String,
    /// The item linked to.
    pub target: String,
    /// `blocks`, `relates_to`, `discovered_from`, `duplicates`,
    /// `supersedes` or `replies_to`.
    pub link_type: String,
}

pub struct WorkItemLinked;
impl EventType for WorkItemLinked {
    const TYPE: &'static str = "work_item.linked";
    const V: u32 = 1;
    type Payload = WorkItemLinkedV1;
}

/// `work_item.commented@1`: a comment on a work item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemCommentedV1 {
    pub work_item: String,
    /// The comment (oxplow: `task_note:<id>`).
    pub comment: String,
}

pub struct WorkItemCommented;
impl EventType for WorkItemCommented {
    const TYPE: &'static str = "work_item.commented";
    const V: u32 = 1;
    type Payload = WorkItemCommentedV1;
}

/// `work_item.recorded@1`: a provider's item as it now stands — how an
/// external provider's items reach `work_item` (the `work_items.project`
/// consumer upserts it by ref). oxplow's own tasks don't log it: their
/// rows are written with the task, in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkItemRecordedV1 {
    pub item: crate::work_items::WorkItemRecord,
}

pub struct WorkItemRecorded;
impl EventType for WorkItemRecorded {
    const TYPE: &'static str = "work_item.recorded";
    const V: u32 = 1;
    type Payload = WorkItemRecordedV1;
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

/// `code.diagnostics.changed@1`: what a file's language servers report
/// about it changed (a publish, or a server gone and its reports with
/// it). Logged debounced, once per file per burst, with the counts after
/// it; `v_diagnostic` has the rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeDiagnosticsChangedV1 {
    /// `stream:<id>`.
    pub stream: String,
    /// Workspace-relative.
    pub path: String,
    pub counts: DiagnosticCounts,
}

pub struct CodeDiagnosticsChanged;
impl EventType for CodeDiagnosticsChanged {
    const TYPE: &'static str = "code.diagnostics.changed";
    const V: u32 = 1;
    type Payload = CodeDiagnosticsChangedV1;
}

/// `plugin.disabled@1` (P7.C1): one of a plugin's contributions — a
/// provider instance, a collector — was stopped on this machine after
/// repeated failures (or a provider whose handshake no longer matches what
/// was approved), and stays off until a person enables it again
/// (`plugin.enable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginDisabledV1 {
    /// `plugin:<extension>`.
    pub plugin: String,
    /// The contribution within it: a provider's id, a collector's id.
    pub contribution: String,
    /// `provider` or `collector`.
    pub kind: String,
    /// What stopped it.
    pub reason: String,
}

pub struct PluginDisabled;
impl EventType for PluginDisabled {
    const TYPE: &'static str = "plugin.disabled";
    const V: u32 = 1;
    type Payload = PluginDisabledV1;
}

/// `plugin.enabled@1` (P7.C1): a person enabled a disabled contribution
/// again on this machine (`plugin.enable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginEnabledV1 {
    /// `plugin:<extension>`.
    pub plugin: String,
    pub contribution: String,
    /// `provider` or `collector`.
    pub kind: String,
}

pub struct PluginEnabled;
impl EventType for PluginEnabled {
    const TYPE: &'static str = "plugin.enabled";
    const V: u32 = 1;
    type Payload = PluginEnabledV1;
}

/// `knowledge.page.written@1`: a knowledge page's row and edges were
/// restated from its body — by `knowledge.write_page` / `link` /
/// `resync`, or by the wiki watcher after a hand edit
/// (`system:wiki_watch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageWrittenV1 {
    /// `wiki:<slug>`.
    pub page: String,
    /// Every ref the page now points at (`file:src/lib.rs`, `wiki:x`, …).
    pub outbound: Vec<String>,
    /// The snapshot its new refs are pinned to (`snap:<id>`), when there
    /// is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

pub struct KnowledgePageWritten;
impl EventType for KnowledgePageWritten {
    const TYPE: &'static str = "knowledge.page.written";
    const V: u32 = 1;
    type Payload = KnowledgePageWrittenV1;
}

/// `lens.shown@1`: an agent showed the person an answer in a thread —
/// an existing lens or its own lens spec (`lens.show`, P6.C1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensShownV1 {
    /// `answer:<id>`.
    pub answer: String,
    /// `thread:<id>`.
    pub thread: String,
    /// `lens:<extension>/<slug>` when it shows an existing lens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
}

pub struct LensShown;
impl EventType for LensShown {
    const TYPE: &'static str = "lens.shown";
    const V: u32 = 1;
    type Payload = LensShownV1;
}

/// `lens.kept@1`: an answer was kept — written as a private lens
/// (`lens.keep`, P6.C1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensKeptV1 {
    /// `answer:<id>`.
    pub answer: String,
    /// `lens:<extension>/<slug>`.
    pub lens: String,
}

pub struct LensKept;
impl EventType for LensKept {
    const TYPE: &'static str = "lens.kept";
    const V: u32 = 1;
    type Payload = LensKeptV1;
}

/// `knowledge.page.deleted@1`: a knowledge page is gone, its row and
/// edges with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageDeletedV1 {
    /// `wiki:<slug>`.
    pub page: String,
}

pub struct KnowledgePageDeleted;
impl EventType for KnowledgePageDeleted {
    const TYPE: &'static str = "knowledge.page.deleted";
    const V: u32 = 1;
    type Payload = KnowledgePageDeletedV1;
}

/// `knowledge.note.written@1`: a thread note was added or its body
/// changed (P7.B6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeNoteWrittenV1 {
    /// `task_note:not<n>`.
    pub note: String,
    /// The thread it's on (`thread:thr<n>`).
    pub thread: String,
}

pub struct KnowledgeNoteWritten;
impl EventType for KnowledgeNoteWritten {
    const TYPE: &'static str = "knowledge.note.written";
    const V: u32 = 1;
    type Payload = KnowledgeNoteWrittenV1;
}

/// `knowledge.note.deleted@1`: a thread note is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeNoteDeletedV1 {
    /// `task_note:not<n>`.
    pub note: String,
    /// The thread it was on (`thread:thr<n>`).
    pub thread: String,
}

pub struct KnowledgeNoteDeleted;
impl EventType for KnowledgeNoteDeleted {
    const TYPE: &'static str = "knowledge.note.deleted";
    const V: u32 = 1;
    type Payload = KnowledgeNoteDeletedV1;
}

/// `knowledge.comment.written@1`: a comment (a threaded annotation on a
/// page) was created or changed — a reply, its intent, status or anchor
/// (P7.B6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCommentWrittenV1 {
    /// `comment:cmt<n>`.
    pub comment: String,
    /// What it annotates: the target's kind (`file`, `work_item`, …) and id.
    pub target_kind: String,
    pub target_id: String,
}

pub struct KnowledgeCommentWritten;
impl EventType for KnowledgeCommentWritten {
    const TYPE: &'static str = "knowledge.comment.written";
    const V: u32 = 1;
    type Payload = KnowledgeCommentWrittenV1;
}

/// `knowledge.comment.deleted@1`: a comment is gone, its messages with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCommentDeletedV1 {
    /// `comment:cmt<n>`.
    pub comment: String,
    pub target_kind: String,
    pub target_id: String,
}

pub struct KnowledgeCommentDeleted;
impl EventType for KnowledgeCommentDeleted {
    const TYPE: &'static str = "knowledge.comment.deleted";
    const V: u32 = 1;
    type Payload = KnowledgeCommentDeletedV1;
}

/// `effort.finished@1`: an effort's close is fully handled — its end
/// snapshot pinned, unclaimed work reconciled, lifecycle metrics projected.
/// Logged once per effort by the effort-lifecycle consumer (dedupe key
/// `effort.finished:<effort>`), caused by the `effort.closed` it handled;
/// what the effort reactors (evidence, inferred decisions) and the
/// `effort.finished` collectors consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffortFinishedV1 {
    pub effort: String,
    pub work_item: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_snapshot: Option<String>,
    /// Recorded after the fact; there was no bracket to snapshot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retroactive: bool,
}

pub struct EffortFinished;
impl EventType for EffortFinished {
    const TYPE: &'static str = "effort.finished";
    const V: u32 = 1;
    type Payload = EffortFinishedV1;
}

/// Whether `event_type` is one of core's types (any version) — what a
/// collector's `on:` trigger may name today. Built once.
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
    fn core_registry_knows_every_core_type_and_version() {
        let r = EventSchemaRegistry::core();
        let all = r.versions();
        let versions: Vec<(&str, u32)> = all.iter().map(|(t, v)| (t.as_str(), *v)).collect();
        assert_eq!(
            versions,
            vec![
                ("agent.prompt.submitted", 1),
                ("agent.session.ended", 1),
                ("agent.session.started", 1),
                ("agent.status.changed", 1),
                ("agent.tool.finished", 1),
                ("agent.tool.requested", 1),
                ("agent.turn.ended", 1),
                ("agent.turn.ended", 2),
                ("agent.turn.started", 1),
                ("code.diagnostics.changed", 1),
                ("collector.synced", 1),
                ("command.approved", 1),
                ("command.declined", 1),
                ("command.executed", 1),
                ("command.proposed", 1),
                ("config.changed", 1),
                ("effect.result", 1),
                ("effort.claim_verified", 1),
                ("effort.closed", 1),
                ("effort.decision_reviewed", 1),
                ("effort.finished", 1),
                ("effort.opened", 1),
                ("knowledge.comment.deleted", 1),
                ("knowledge.comment.written", 1),
                ("knowledge.note.deleted", 1),
                ("knowledge.note.written", 1),
                ("knowledge.page.deleted", 1),
                ("knowledge.page.written", 1),
                ("lens.kept", 1),
                ("lens.shown", 1),
                ("plugin.disabled", 1),
                ("plugin.enabled", 1),
                ("snapshot.taken", 1),
                ("test.coverage.recorded", 1),
                ("test.run.recorded", 1),
                ("vcs.head.moved", 1),
                ("work_item.commented", 1),
                ("work_item.created", 1),
                ("work_item.deleted", 1),
                ("work_item.edited", 1),
                ("work_item.linked", 1),
                ("work_item.recorded", 1),
                ("work_item.transitioned", 1),
            ]
        );
        assert_eq!(r.latest("work_item.transitioned"), Some(1));
        assert_eq!(r.latest("agent.turn.ended"), Some(2));
        assert_eq!(r.owner("config.changed", 1), Some(None));
    }

    /// P3.1 (tsk471): the first versioned type. A `turn.ended` written at
    /// v1 reads at v2 with no transcript and no usage; the v2 producer's
    /// shape is what `Envelope::typed` emits.
    #[test]
    fn turn_ended_v1_upcasts_to_v2_with_nothing_added() {
        let r = crate::vocabulary::Vocabulary::core();
        let v1 = json!({ "turn": "turn:trn12", "thread": "thread:thr3", "outcome": "completed" });
        let (v, up) = r
            .upcast_to_latest("agent.turn.ended", 1, v1.clone())
            .unwrap();
        assert_eq!(v, 2);
        assert_eq!(up, v1, "optional fields are absent, not null");
        let typed: AgentTurnEndedV2 = serde_json::from_value(up).unwrap();
        assert_eq!(typed.transcript_path, None);
        assert_eq!(typed.usage, None);
        let env = Envelope::typed::<AgentTurnEnded>(
            "test",
            &AgentTurnEndedV2 {
                turn: "turn:trn12".into(),
                thread: "thread:thr3".into(),
                outcome: crate::hook::TurnOutcome::Completed,
                transcript_path: Some("/tmp/t.jsonl".into()),
                usage: Some(TurnUsage {
                    input: 10,
                    output: 4,
                    cache_write: 0,
                    cache_read: 6,
                    model: Some("claude".into()),
                }),
            },
        );
        assert_eq!(env.v, 2);
        r.validate_envelope(&env).unwrap();
        // v1 stays a registered, validating shape for the rows already written.
        r.validate("agent.turn.ended", 1, &v1).unwrap();
    }

    #[test]
    fn validate_accepts_the_typed_shape_and_names_the_violation() {
        let r = crate::vocabulary::Vocabulary::core();
        let ok = Envelope::typed::<WorkItemTransitioned>(
            "human",
            &WorkItemTransitionedV1 {
                work_item: "work_item:oxplow:tsk4".into(),
                from: TaskStatus::Ready,
                to: TaskStatus::InProgress,
                effort: Some("effort:eff9".into()),
            },
        );
        assert_eq!(ok.event_type, "work_item.transitioned");
        assert_eq!(ok.v, 1);
        r.validate_envelope(&ok).unwrap();

        let bad = r
            .validate(
                "work_item.transitioned",
                1,
                &json!({"work_item": "x", "from": "ready", "to": "flying"}),
            )
            .unwrap_err();
        let msg = bad.to_string();
        assert!(
            msg.contains("work_item.transitioned@1") && msg.contains("/to"),
            "{msg}"
        );

        let extra = r
            .validate(
                "config.changed",
                1,
                &json!({"key": "zones", "before": null, "after": 1, "oops": 1}),
            )
            .unwrap_err();
        assert!(extra.to_string().contains("oops"), "{extra}");
    }

    #[test]
    fn unknown_types_and_versions_are_refused() {
        let r = EventSchemaRegistry::core();
        let e = r
            .validate("work_item.transitioned", 2, &json!({}))
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
        type Payload = ConfigChangedV1;
    }
    struct Squatter;
    impl EventType for Squatter {
        const TYPE: &'static str = "work_item.hijacked";
        const V: u32 = 1;
        type Payload = ConfigChangedV1;
    }

    #[test]
    fn a_plugin_declares_types_only_under_its_own_namespace() {
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
            .register_declared("other-plugin", declared::<AcmeDecided>())
            .unwrap_err();
        assert!(foreign.to_string().contains("other_plugin.*"), "{foreign}");
        let dup = r
            .register_declared("acme-review", declared::<AcmeDecided>())
            .unwrap_err();
        assert!(dup.to_string().contains("already registered"), "{dup}");
        // Core can't register into a plugin namespace either, and a
        // second registration of the same type@v collides.
        assert!(r.register::<AcmeDecided>().is_err());
        assert!(r.register::<WorkItemTransitioned>().is_err());
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
        assert!(WorkItemTransitioned::upcast(0, json!({})).is_err());
    }
}
