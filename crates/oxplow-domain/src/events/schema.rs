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
//! `.context/target-architecture.md` §5.3 ([`CORE_NAMESPACES`]). A plugin
//! registers types only under its own name as namespace; registering
//! into a core namespace, or under another plugin's, is refused.

use std::collections::HashMap;
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

type Upcast = Arc<dyn Fn(u32, Value) -> Result<Value, DomainError> + Send + Sync>;

struct Registered {
    validator: jsonschema::Validator,
    schema: Value,
    /// Who registered it: `None` for core, `Some(plugin)` otherwise.
    owner: Option<String>,
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
        r.register::<AgentTurnEnded>().expect("core type registers");
        r.register::<EffortOpened>().expect("core type registers");
        r.register::<EffortClosed>().expect("core type registers");
        r.register::<EffortFinished>().expect("core type registers");
        r
    }

    /// Register a core type. Its namespace must be a core namespace.
    pub fn register<T: EventType>(&mut self) -> Result<(), DomainError> {
        let ns = namespace_of(T::TYPE);
        if !CORE_NAMESPACES.contains(&ns) {
            return Err(DomainError::Invalid(format!(
                "`{}` is not in a core namespace; a plugin registers it with `register_plugin`",
                T::TYPE
            )));
        }
        self.insert::<T>(None)
    }

    /// Register a plugin's type. Its namespace must be the plugin's own
    /// name — never a core namespace, never another plugin's.
    pub fn register_plugin<T: EventType>(&mut self, plugin: &str) -> Result<(), DomainError> {
        let ns = namespace_of(T::TYPE);
        if CORE_NAMESPACES.contains(&ns) {
            return Err(DomainError::Invalid(format!(
                "plugin `{plugin}` may not register `{}`: `{ns}` is a core namespace",
                T::TYPE
            )));
        }
        if ns != plugin {
            return Err(DomainError::Invalid(format!(
                "plugin `{plugin}` may only register types under `{plugin}.*`, not `{}`",
                T::TYPE
            )));
        }
        self.insert::<T>(Some(plugin.to_string()))
    }

    fn insert<T: EventType>(&mut self, owner: Option<String>) -> Result<(), DomainError> {
        validate_type_name(T::TYPE)?;
        let key = (T::TYPE.to_string(), T::V);
        if self.by_version.contains_key(&key) {
            return Err(DomainError::Invalid(format!(
                "event type `{}@{}` is already registered",
                T::TYPE,
                T::V
            )));
        }
        let schema = schema_for::<T>();
        let validator = jsonschema::validator_for(&schema)
            .map_err(|e| DomainError::Invariant(format!("schema for `{}`: {e}", T::TYPE)))?;
        self.by_version.insert(
            key,
            Registered {
                validator,
                schema,
                owner,
            },
        );
        let latest = self.latest.entry(T::TYPE.to_string()).or_insert(0);
        if T::V >= *latest {
            *latest = T::V;
            self.upcasts.insert(
                T::TYPE.to_string(),
                Arc::new(|from, payload| T::upcast(from, payload)),
            );
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

    /// Who registered `type@v`: `Some(None)` for core, `Some(Some(plugin))`
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

    /// Validate an envelope's `type@v` and payload.
    /// The payload against its schema, and every subject as a canonical
    /// ref of a registered kind (a consumer resolves subjects; a malformed
    /// one would dead-letter far from the producer that wrote it).
    pub fn validate_envelope(&self, env: &Envelope) -> Result<(), DomainError> {
        self.validate(&env.event_type, env.v, &env.payload)?;
        for subject in &env.subject {
            crate::refs::validate_ref(subject)
                .map_err(|e| DomainError::Invalid(format!("{} subject: {e}", env.event_type)))?;
        }
        Ok(())
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
            let up = self.upcasts.get(event_type).expect("latest has an upcast");
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

/// `agent.turn.ended@1`: a turn closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnEndedV1 {
    pub turn: String,
    pub thread: String,
    pub outcome: crate::hook::TurnOutcome,
}

pub struct AgentTurnEnded;
impl EventType for AgentTurnEnded {
    const TYPE: &'static str = "agent.turn.ended";
    const V: u32 = 1;
    type Payload = AgentTurnEndedV1;
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

pub struct EffortClosed;
impl EventType for EffortClosed {
    const TYPE: &'static str = "effort.closed";
    const V: u32 = 1;
    type Payload = EffortClosedV1;
}

/// `effort.finished@1`: an effort's close is fully handled — its end
/// snapshot pinned, unclaimed work reconciled, lifecycle metrics projected.
/// Logged once per effort by the effort-lifecycle consumer (dedupe key
/// `effort.finished:<effort>`), caused by the `effort.closed` it handled;
/// what the effort reactors (evidence, inferred decisions, gauges) consume.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn core_registry_knows_every_core_type_at_v1() {
        let r = EventSchemaRegistry::core();
        assert_eq!(
            r.versions(),
            vec![
                ("agent.turn.ended".to_string(), 1),
                ("agent.turn.started".to_string(), 1),
                ("command.executed".to_string(), 1),
                ("config.changed".to_string(), 1),
                ("effect.result".to_string(), 1),
                ("effort.closed".to_string(), 1),
                ("effort.finished".to_string(), 1),
                ("effort.opened".to_string(), 1),
                ("snapshot.taken".to_string(), 1),
                ("vcs.head.moved".to_string(), 1),
                ("work_item.transitioned".to_string(), 1),
            ]
        );
        assert_eq!(r.latest("work_item.transitioned"), Some(1));
        assert_eq!(r.owner("config.changed", 1), Some(None));
    }

    #[test]
    fn validate_accepts_the_typed_shape_and_names_the_violation() {
        let r = EventSchemaRegistry::core();
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
    fn plugins_register_only_under_their_own_namespace() {
        let mut r = EventSchemaRegistry::core();
        r.register_plugin::<AcmeDecided>("acme_review").unwrap();
        assert_eq!(r.owner("acme_review.decided", 1), Some(Some("acme_review")));
        assert!(r.register_plugin::<Squatter>("acme_review").is_err());
        assert!(r.register_plugin::<AcmeDecided>("other_plugin").is_err());
        // Core can't register into a plugin namespace either, and a
        // second registration of the same type@v collides.
        assert!(r.register::<AcmeDecided>().is_err());
        assert!(r.register::<WorkItemTransitioned>().is_err());
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
        r.register_plugin::<Thing1>("acme").unwrap();
        r.register_plugin::<Thing2>("acme").unwrap();
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
