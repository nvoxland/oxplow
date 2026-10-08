//! Commands: the one write path (`.context/commands.md`). Pure data here — the spec a command
//! declares, who is running it, and what running it produced. The bus
//! that executes them lives in `oxplow-app`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::events::schema::ActorKind;
use crate::events::validate_type_name;
use crate::ids::{StreamId, ThreadId};
use crate::{DomainError, EventId};

/// Which surfaces may invoke a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Invokers {
    pub human: bool,
    pub agent: bool,
    pub lens: bool,
}

impl Invokers {
    pub const ALL: Invokers = Invokers {
        human: true,
        agent: true,
        lens: true,
    };
    pub const HUMAN_ONLY: Invokers = Invokers {
        human: true,
        agent: false,
        lens: false,
    };
    /// A person, directly or through a lens — never an agent.
    pub const NO_AGENT: Invokers = Invokers {
        human: true,
        agent: false,
        lens: true,
    };

    pub fn allows(&self, invoker: Invoker) -> bool {
        match invoker {
            Invoker::Human => self.human,
            Invoker::Agent => self.agent,
            Invoker::Lens => self.lens,
        }
    }

    /// Whether this admits no one `floor` doesn't: a declaration narrows
    /// an operation's floor, never widens it.
    pub fn within(&self, floor: &Invokers) -> bool {
        (!self.human || floor.human) && (!self.agent || floor.agent) && (!self.lens || floor.lens)
    }

    /// Who it admits, for a message: `human, lens`; `no one`.
    pub fn names(&self) -> String {
        let names: Vec<&str> = [
            (self.human, "human"),
            (self.agent, "agent"),
            (self.lens, "lens"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        if names.is_empty() {
            "no one".into()
        } else {
            names.join(", ")
        }
    }
}

/// The surface a run comes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Invoker {
    Human,
    Agent,
    Lens,
}

/// Whether a run must be confirmed by a person first. An agent can never
/// confirm: it receives `NeedsConfirmation` and writes nothing. Ordered
/// weakest first, so an operation's floor compares: a declaration may
/// ask more than its operation's `confirm_at_least`, never less.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Confirm {
    Never,
    Always,
    /// Irreversible; presented as destructive and always confirmed.
    Destructive,
}

impl Confirm {
    pub fn required(&self) -> bool {
        !matches!(self, Confirm::Never)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Stable,
    Experimental,
}

/// Whether the handler runs inside the bus's transaction (with the audit
/// row and `command.executed`), or outside it, against a system the bus
/// doesn't own — a VCS, a provider process, a collector's script — and is
/// audited after it returns (`External`). A `Dispatch` command decides
/// per input (the `work_item.*` verbs: oxplow's own items in the
/// transaction, another provider's through its process) and then runs
/// exactly as one or the other. A test lists the `External` and the
/// `Dispatch` commands, so each one is a reviewed choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Atomicity {
    Tx,
    External,
    Dispatch,
}

/// Whether a command changes anything, and who may. A `Read` runs
/// without an audit row or a `command.executed` event (a polling agent
/// must not fill the log), and an agent thread that may not write can
/// still run it. A `Write` is refused outright to an agent thread that
/// isn't its stream's writer. A `Record` changes oxplow's own records
/// (filing and editing tasks) and is audited like a `Write`, but any
/// thread may run it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandEffect {
    Read,
    #[default]
    Write,
    Record,
}

/// What a command declares about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct CommandSpec {
    /// The command id: `<namespace>.<area>.<verb>`, snake_case —
    /// `oxplow.work_item.transition`, `acme_pr.issue.close`. The namespace
    /// is its owner's (`oxplow` for core and oxplow's own extensions, an
    /// extension's declared `namespace:` otherwise), so two extensions'
    /// areas never collide. (A person reads the command's label, not its
    /// id.)
    pub id: String,
    /// One sentence for `list_commands` and the launcher.
    pub summary: String,
    /// JSON Schema for the input.
    #[specta(type = crate::Json)]
    pub input_schema: Value,
    pub invokers: Invokers,
    pub confirm: Confirm,
    /// The handler returns an inverse, so `commands.undo` can apply it.
    pub undoable: bool,
    pub lifecycle: Lifecycle,
    pub atomicity: Atomicity,
    pub effect: CommandEffect,
    /// The scopes its handler calls (`sql.read`) and the capabilities, or
    /// their features (`work_items.comments`), it needs active; unmet, it
    /// isn't offered and doesn't run (`.context/commands.md`).
    pub needs: Vec<String>,
    /// How a person meets it — label, group, where it's offered and how
    /// it runs from there. `None`: it isn't offered to a person by itself
    /// (a step other commands compose, an agent's tool).
    #[serde(default)]
    pub ui: Option<CommandUi>,
    /// The scope operation behind it, when it is one: where it runs (a
    /// window scope's runs in the window) and what it does.
    #[serde(default)]
    pub op: Option<OpRef>,
    /// Top-level input fields its record leaves out — a file's content —
    /// kept as their size (`{ "omitted_bytes": n }`): the run is still
    /// audited, the audit log doesn't grow by every file saved.
    #[serde(default)]
    pub unrecorded: Vec<String>,
}

impl CommandSpec {
    /// `input` as its record keeps it: each [`Self::unrecorded`] field
    /// replaced by its size.
    pub fn recorded_input(&self, input: &Value) -> Value {
        let mut out = input.clone();
        if let Some(o) = out.as_object_mut() {
            for field in &self.unrecorded {
                if let Some(v) = o.get_mut(field) {
                    let bytes = match &*v {
                        Value::String(s) => s.len(),
                        other => other.to_string().len(),
                    };
                    *v = serde_json::json!({ "omitted_bytes": bytes });
                }
            }
        }
        out
    }
}

/// An operation of a scope (`scope: tabs.write`, `op: open`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct OpRef {
    pub scope: String,
    pub op: String,
}

/// A command as a person meets it (`.context/commands.md` "Offering a
/// command to a person"). Agents ignore it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandUi {
    /// What a person reads: `Pull Changes`, `New Task…`.
    pub label: String,
    /// What search lists it under: `Git`, `Tasks`.
    #[serde(default)]
    pub group: Option<String>,
    /// More words search matches it by.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// The ref kind it acts on (`work_item`, `effort`): offered on that
    /// ref's page and rows. `None`: it needs no ref, and search offers it.
    #[serde(default)]
    pub about: Option<String>,
    /// The input it runs with. A string that is exactly `{{stream}}`,
    /// `{{thread}}`, `{{ref}}` or `{{ref.id}}` is bound from where it runs;
    /// a binding there's nothing for makes it unavailable there.
    #[serde(default)]
    #[specta(type = Option<crate::Json>)]
    pub input: Option<Value>,
    /// What gathers its input — a page (a tab id, `page:new-task`) or one
    /// of the window's own forms (`new-thread`, `commit`): choosing it
    /// opens that rather than running it.
    #[serde(default)]
    pub form: Option<String>,
    /// The page to open once it ran: a tab id whose `{{result.<field>}}`
    /// takes the result's field (`page:custom-dashboard?id={{result.id}}`).
    #[serde(default)]
    pub open_after: Option<String>,
    /// It runs as a background task (a slow call out: pull, push) whose
    /// failure is reported, rather than awaited where it was chosen.
    #[serde(default)]
    pub background: bool,
    /// The key that runs it, as `Ctrl/Cmd+S` / `Ctrl/Cmd+Shift+N`
    /// (`Ctrl/Cmd` is Cmd on macOS, Ctrl elsewhere).
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Its shortcut runs it while the person types in a field too (Save,
    /// Find); otherwise typing keeps it.
    #[serde(default)]
    pub while_typing: bool,
    /// Where the menu bar shows it.
    #[serde(default)]
    pub menu: Option<MenuPlace>,
    /// When it's offered — in search, the menu bar and by its shortcut —
    /// in VS Code's when-clause syntax over the window's context keys
    /// (`crate::when`): `fileShown && fileDirty`, `streamKind == worktree`.
    /// Absent, always.
    #[serde(default)]
    pub when: Option<String>,
}

/// A command's place in the menu bar: which menu, the group it sits in
/// (VS Code's: groups are sorted by name and drawn apart by a separator)
/// and where in the group (lowest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MenuPlace {
    /// `file` or `edit`.
    pub bar: String,
    /// Its group (`1_project`, `2_save`); absent, the menu's first.
    #[serde(default)]
    pub group: Option<String>,
    pub order: u32,
}

/// The namespace oxplow's own commands are under: core's and its shipped
/// extensions' (`oxplow-bundled`, …) alike, so a command moving between
/// them keeps its id.
pub const OXPLOW_NAMESPACE: &str = "oxplow";

/// The namespace of a command id — its first segment.
pub fn namespace_of(id: &str) -> &str {
    id.split('.').next().unwrap_or_default()
}

impl CommandSpec {
    /// A command id is `<namespace>.<area>.<verb>`: exactly three
    /// snake_case segments.
    pub fn validate_id(id: &str) -> Result<(), DomainError> {
        let three = id.split('.').count() == 3;
        if three && validate_type_name(id).is_ok() {
            return Ok(());
        }
        Err(DomainError::Invalid(format!(
            "command id `{id}` must be `<namespace>.<area>.<verb>` in snake_case \
             (e.g. `oxplow.work_item.transition`)"
        )))
    }
}

/// Who is running a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    Human,
    Agent {
        /// The agent's thread, when the transport carried it (the MCP
        /// identity middleware supplies it from `X-Oxplow-Thread`).
        thread_id: Option<ThreadId>,
        stream_id: Option<StreamId>,
    },
    Lens {
        lens_id: String,
        on_behalf_of: Box<Actor>,
    },
    System,
    /// An extension's effect reacting to an event (`effects:`, P8.D8):
    /// an agent's invoker rights, no thread, and never a confirmation —
    /// a command that asks becomes a proposal for a person.
    Effect {
        /// `<extension>/<effect id>`.
        effect: String,
    },
}

impl Actor {
    pub fn kind(&self) -> ActorKind {
        match self {
            Actor::Human => ActorKind::Human,
            Actor::Agent { .. } => ActorKind::Agent,
            Actor::Lens { .. } => ActorKind::Lens,
            Actor::System => ActorKind::System,
            Actor::Effect { .. } => ActorKind::Effect,
        }
    }

    /// Whether this actor may confirm a run that asks: a person, directly
    /// or through a lens they used. An agent, an effect and oxplow itself
    /// never can.
    pub fn may_confirm(&self) -> bool {
        match self {
            Actor::Human => true,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.may_confirm(),
            Actor::Agent { .. } | Actor::System | Actor::Effect { .. } => false,
        }
    }

    /// Whether a run of this actor's that asks is kept as a proposal for
    /// a person (an agent's, an effect's) rather than asking it directly.
    pub fn proposes(&self) -> bool {
        match self {
            Actor::Agent { .. } | Actor::Effect { .. } => true,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.proposes(),
            Actor::Human | Actor::System => false,
        }
    }

    /// An agent, or a lens acting for one (at any depth). Agent-only
    /// rules — the agent policy, "can never confirm" — apply to both.
    pub fn is_agent_driven(&self) -> bool {
        match self {
            Actor::Agent { .. } => true,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.is_agent_driven(),
            Actor::Human | Actor::System | Actor::Effect { .. } => false,
        }
    }

    /// The agent behind this actor, if any (see [`Self::is_agent_driven`]).
    pub fn agent_thread(&self) -> Option<Option<ThreadId>> {
        match self {
            Actor::Agent { thread_id, .. } => Some(*thread_id),
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.agent_thread(),
            Actor::Human | Actor::System | Actor::Effect { .. } => None,
        }
    }

    pub fn invoker(&self) -> Invoker {
        match self {
            Actor::Human | Actor::System => Invoker::Human,
            Actor::Agent { .. } | Actor::Effect { .. } => Invoker::Agent,
            Actor::Lens { .. } => Invoker::Lens,
        }
    }

    /// The `source` an event or audit row records: `human`, `agent:thr3`,
    /// `lens:acme/blocked`, `system`, `effect:acme/notify`.
    pub fn source(&self) -> String {
        match self {
            Actor::Human => "human".into(),
            Actor::Agent {
                thread_id: Some(t), ..
            } => format!("agent:{t}"),
            Actor::Agent {
                thread_id: None, ..
            } => "agent".into(),
            Actor::Lens { lens_id, .. } => format!("lens:{lens_id}"),
            Actor::System => "system".into(),
            Actor::Effect { effect } => format!("effect:{effect}"),
        }
    }

    /// The id part of `source`, for the audit's `actor_id`.
    pub fn id(&self) -> Option<String> {
        match self {
            Actor::Agent {
                thread_id: Some(t), ..
            } => Some(t.to_string()),
            Actor::Lens { lens_id, .. } => Some(lens_id.clone()),
            Actor::Effect { effect } => Some(effect.clone()),
            _ => None,
        }
    }

    pub fn thread_id(&self) -> Option<ThreadId> {
        match self {
            Actor::Agent { thread_id, .. } => *thread_id,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.thread_id(),
            _ => None,
        }
    }

    /// The agent's stream, when the transport carried it (through a lens
    /// chain too, like [`Self::thread_id`]).
    pub fn stream_id(&self) -> Option<StreamId> {
        match self {
            Actor::Agent { stream_id, .. } => *stream_id,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.stream_id(),
            _ => None,
        }
    }

    /// The anchors every event this actor causes carries.
    pub fn anchors(&self) -> crate::events::Anchors {
        crate::events::Anchors {
            thread_id: self.thread_id(),
            stream_id: self.stream_id(),
            ..crate::events::Anchors::default()
        }
    }
}

/// A command by name with its input — what an inverse is, and what
/// `run_command` receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommandCall {
    pub name: String,
    #[specta(type = crate::Json)]
    pub input: Value,
}

/// What a confirmation prompt shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Preview {
    pub command: String,
    pub summary: String,
    pub input: Value,
    pub destructive: bool,
}

/// A completed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommandOutcome {
    #[specta(type = crate::Json)]
    pub result: Value,
    /// The `command_audit` row; `None` for a `Read` command.
    pub audit_id: Option<i64>,
    /// The `command.executed` event; `None` for a `Read` command.
    pub event_id: Option<EventId>,
    /// Present when the command is undoable: `commands.undo(audit_id)`
    /// runs it.
    pub inverse: Option<CommandCall>,
}

/// Why a run did not complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandError {
    /// No command of that name (for this actor's view).
    Unknown { name: String },
    /// The input failed the schema; `field` is the JSON pointer when known.
    Invalid {
        field: Option<String>,
        message: String,
    },
    /// The invoker or the agent policy refused it.
    Denied { reason: String },
    /// A person has to confirm first. Nothing was written. Boxed so the
    /// error stays small on the hot `Result` paths.
    NeedsConfirmation { preview: Box<Preview> },
    /// An agent's run needed a person's confirmation, so it was kept as a
    /// proposal (`proposal:<id>`) for a person to approve or decline.
    /// Nothing ran; nothing else was written.
    Proposed {
        proposal: String,
        preview: Box<Preview>,
        /// The pending proposals of the same call it replaced.
        supersedes: Vec<String>,
    },
    /// The handler failed.
    Failed { message: String },
    /// The system outside oxplow it reached couldn't answer just now — it
    /// erred, timed out, or asked to wait (`retry_after_ms`): the one
    /// failure worth sending again by itself (tsk914). Whatever else fails
    /// is `Failed` and waits for a person.
    Unavailable {
        message: String,
        retry_after_ms: Option<u64>,
    },
    /// The database stayed busy through the run's retries; nothing was
    /// written. Worth retrying.
    Busy { message: String },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Unknown { name } => write!(f, "unknown command `{name}`"),
            CommandError::Invalid {
                field: Some(p),
                message,
            } => write!(f, "invalid input at `{p}`: {message}"),
            CommandError::Invalid {
                field: None,
                message,
            } => write!(f, "invalid input: {message}"),
            CommandError::Denied { reason } => write!(f, "denied: {reason}"),
            CommandError::NeedsConfirmation { preview } => {
                write!(f, "`{}` needs confirmation", preview.command)
            }
            CommandError::Proposed {
                proposal,
                preview,
                supersedes,
            } => {
                write!(
                    f,
                    "`{}` needs a person's approval; it is recorded as {proposal}",
                    preview.command
                )?;
                if !supersedes.is_empty() {
                    write!(f, " (replaces {})", supersedes.join(", "))?;
                }
                Ok(())
            }
            CommandError::Failed { message } => write!(f, "command failed: {message}"),
            CommandError::Unavailable { message, .. } => {
                write!(f, "command failed: {message}")
            }
            CommandError::Busy { message } => {
                write!(f, "database busy, nothing written: {message}")
            }
        }
    }
}

impl std::error::Error for CommandError {}

impl From<DomainError> for CommandError {
    fn from(e: DomainError) -> Self {
        match e {
            DomainError::Invalid(m) => CommandError::Invalid {
                field: None,
                message: m,
            },
            DomainError::Busy(m) => CommandError::Busy { message: m },
            other => CommandError::Failed {
                message: other.to_string(),
            },
        }
    }
}

/// A compiled input schema. Wraps the validator so the bus needs no
/// schema-library dependency of its own.
pub struct InputValidator {
    validator: jsonschema::Validator,
    schema: Value,
}

impl InputValidator {
    pub fn compile(schema: &Value) -> Result<Self, DomainError> {
        let validator = jsonschema::validator_for(schema)
            .map_err(|e| DomainError::Invariant(format!("input schema: {e}")))?;
        Ok(Self {
            validator,
            schema: schema.clone(),
        })
    }

    /// The first violation, as `Invalid { field: <JSON pointer>, … }`,
    /// saying what the schema accepts there (the object's fields, or the
    /// value's choices) so the caller can fix it in one more call.
    pub fn check(&self, input: &Value) -> Result<(), CommandError> {
        use jsonschema::error::ValidationErrorKind as Kind;
        match self.validator.iter_errors(input).next() {
            None => Ok(()),
            Some(err) => {
                let path = err.instance_path().to_string();
                let at = schema_at(&self.schema, &path);
                let accepted = match err.kind() {
                    Kind::AdditionalProperties { .. } | Kind::Required { .. } => {
                        at.and_then(|s| accepted_fields(&self.schema, s))
                    }
                    Kind::AnyOf { .. }
                    | Kind::OneOfNotValid { .. }
                    | Kind::Enum { .. }
                    | Kind::Constant { .. } => at.and_then(|s| accepted_values(&self.schema, s)),
                    _ => None,
                };
                Err(CommandError::Invalid {
                    field: if path.is_empty() { None } else { Some(path) },
                    message: match accepted {
                        Some(a) => format!("{err}. {a}"),
                        None => err.to_string(),
                    },
                })
            }
        }
    }
}

/// `node` with its `$ref`s followed, and an optional field's `T | null`
/// (`anyOf` with one non-null branch) narrowed to `T`.
fn resolved<'a>(root: &'a Value, mut node: &'a Value) -> &'a Value {
    for _ in 0..16 {
        if let Some(r) = node.get("$ref").and_then(Value::as_str) {
            match r.strip_prefix('#').and_then(|p| root.pointer(p)) {
                Some(next) => node = next,
                None => return node,
            }
            continue;
        }
        let branches = node
            .get("anyOf")
            .or_else(|| node.get("oneOf"))
            .and_then(Value::as_array);
        let non_null: Vec<&Value> = branches
            .into_iter()
            .flatten()
            .filter(|b| b.get("type").and_then(Value::as_str) != Some("null"))
            .collect();
        match non_null.as_slice() {
            [only] if branches.is_some_and(|b| b.len() > 1) => node = only,
            _ => return node,
        }
    }
    node
}

/// The subschema describing the value at `instance` (a JSON pointer) in
/// `root`, through `properties` and `items`.
fn schema_at<'a>(root: &'a Value, instance: &str) -> Option<&'a Value> {
    let mut node = resolved(root, root);
    for token in instance.split('/').skip(1) {
        let token = token.replace("~1", "/").replace("~0", "~");
        node = match node.get("properties").and_then(|p| p.get(&token)) {
            Some(prop) => prop,
            None => node.get("items")?,
        };
        node = resolved(root, node);
    }
    Some(node)
}

/// "Accepted: a (required), b, c" for an object schema: the required
/// fields first.
fn accepted_fields(root: &Value, node: &Value) -> Option<String> {
    let node = resolved(root, node);
    let props = node.get("properties")?.as_object()?;
    let required: Vec<&str> = node
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let fields: Vec<String> = required
        .iter()
        .filter(|k| props.contains_key(**k))
        .map(|k| format!("{k} (required)"))
        .chain(
            props
                .keys()
                .filter(|k| !required.contains(&k.as_str()))
                .cloned(),
        )
        .collect();
    Some(format!("Accepted: {}", fields.join(", ")))
}

/// "Expected one of: …" for a schema that is a choice of literal values
/// (`enum`, `const`, or branches of them).
fn accepted_values(root: &Value, node: &Value) -> Option<String> {
    fn collect(root: &Value, node: &Value, out: &mut Vec<String>) {
        let node = resolved(root, node);
        if let Some(values) = node.get("enum").and_then(Value::as_array) {
            out.extend(values.iter().map(Value::to_string));
        }
        if let Some(value) = node.get("const") {
            out.push(value.to_string());
        }
        for key in ["anyOf", "oneOf"] {
            for branch in node
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                collect(root, branch, out);
            }
        }
    }
    let mut values = Vec::new();
    collect(root, node, &mut values);
    values.dedup();
    (!values.is_empty()).then(|| format!("Expected one of: {}", values.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A refused input says what would have been accepted there, so a
    /// caller fixes it in one more call instead of one field at a time.
    #[test]
    fn an_invalid_input_names_what_is_accepted_there() {
        #[derive(serde::Deserialize, schemars::JsonSchema)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Create {
            title: String,
            #[serde(default)]
            body: Option<String>,
            #[serde(default)]
            state: Option<crate::work_items::CanonicalState>,
        }
        let schema = serde_json::to_value(schemars::schema_for!(Create)).unwrap();
        let v = InputValidator::compile(&schema).unwrap();
        let message = |input: Value| match v.check(&input) {
            Err(CommandError::Invalid { message, .. }) => message,
            other => panic!("{other:?}"),
        };

        let unknown = message(json!({ "title": "t", "description": "d" }));
        assert!(
            unknown.contains("'description' was unexpected"),
            "{unknown}"
        );
        assert!(
            unknown.contains("Accepted: title (required), body, state"),
            "{unknown}"
        );

        let missing = message(json!({ "body": "b" }));
        assert!(
            missing.contains("Accepted: title (required), body, state"),
            "{missing}"
        );

        let state = message(json!({ "title": "t", "state": "ready" }));
        assert!(
            state.contains(
                r#"Expected one of: "todo", "in_progress", "blocked", "done", "canceled""#
            ),
            "{state}"
        );
    }

    #[test]
    fn actors_know_their_invoker_source_and_kind() {
        let agent = Actor::Agent {
            thread_id: Some(ThreadId::new(3)),
            stream_id: None,
        };
        assert_eq!(agent.invoker(), Invoker::Agent);
        assert_eq!(agent.source(), "agent:thr3");
        assert_eq!(agent.id().as_deref(), Some("thr3"));
        assert_eq!(agent.kind(), ActorKind::Agent);
        let lens = Actor::Lens {
            lens_id: "acme/blocked".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        assert_eq!(lens.invoker(), Invoker::Lens);
        assert_eq!(lens.source(), "lens:acme/blocked");
        assert_eq!(lens.thread_id(), Some(ThreadId::new(3)));
        assert_eq!(Actor::Human.source(), "human");
        assert_eq!(Actor::System.invoker(), Invoker::Human);
        let effect = Actor::Effect {
            effect: "acme/notify".into(),
        };
        assert_eq!(effect.invoker(), Invoker::Agent);
        assert_eq!(effect.source(), "effect:acme/notify");
        assert_eq!(effect.id().as_deref(), Some("acme/notify"));
        assert_eq!(effect.kind(), ActorKind::Effect);
    }

    /// Only a person confirms — directly or through a lens they used.
    #[test]
    fn only_a_person_may_confirm() {
        let agent = Actor::Agent {
            thread_id: None,
            stream_id: None,
        };
        let lens = |on: Actor| Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(on),
        };
        assert!(Actor::Human.may_confirm());
        assert!(lens(Actor::Human).may_confirm());
        for actor in [
            agent.clone(),
            lens(agent),
            Actor::System,
            Actor::Effect {
                effect: "acme/notify".into(),
            },
        ] {
            assert!(!actor.may_confirm(), "{actor:?}");
        }
    }

    #[test]
    fn a_command_id_is_namespace_area_verb() {
        assert!(CommandSpec::validate_id("oxplow.work_item.transition").is_ok());
        assert!(CommandSpec::validate_id("acme_pr.issue.close").is_ok());
        for bad in [
            "work_item.transition",
            "transition",
            "oxplow.work_item.transition.now",
            "Oxplow.work_item.transition",
            "oxplow..transition",
        ] {
            assert!(CommandSpec::validate_id(bad).is_err(), "{bad}");
        }
        assert_eq!(namespace_of("oxplow.work_item.create"), "oxplow");
    }

    #[test]
    fn input_validation_names_the_field() {
        let v = InputValidator::compile(&json!({
            "type": "object",
            "required": ["id"],
            "properties": { "id": { "type": "string" }, "n": { "type": "integer" } },
            "additionalProperties": false
        }))
        .unwrap();
        v.check(&json!({"id": "tsk1"})).unwrap();
        let err = v.check(&json!({"id": "tsk1", "n": "x"})).unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/n"),
            "{err:?}"
        );
        let err = v.check(&json!({})).unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }));
        assert!(err.to_string().contains("id"), "{err}");
    }
}
