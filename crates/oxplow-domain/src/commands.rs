//! Commands: the one write path (`.context/target-architecture.md` §7,
//! `.context/commands.md`). Pure data here — the spec a command
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

    pub fn allows(&self, invoker: Invoker) -> bool {
        match invoker {
            Invoker::Human => self.human,
            Invoker::Agent => self.agent,
            Invoker::Lens => self.lens,
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
/// confirm: it receives `NeedsConfirmation` and writes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
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
/// row and `command.executed`), or is a pre-existing service call with
/// its own transactions that the bus audits after the fact. `BestEffort`
/// exists only for handlers that predate the bus; a test counts them so
/// the number trends to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Atomicity {
    Tx,
    BestEffort,
}

/// Whether a command changes anything. A `Read` runs without an audit
/// row or a `command.executed` event (a polling agent must not fill the
/// log), and an agent thread that may not write can still run it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandEffect {
    Read,
    #[default]
    Write,
}

/// What a command declares about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct CommandSpec {
    /// `<capability|plugin>.<verb>`, snake_case: `work_item.transition`,
    /// `config.set`.
    pub name: String,
    /// One sentence for `list_commands` and the launcher.
    pub summary: String,
    /// JSON Schema for the input.
    pub input_schema: Value,
    pub invokers: Invokers,
    pub confirm: Confirm,
    /// The handler returns an inverse, so `commands.undo` can apply it.
    pub undoable: bool,
    pub lifecycle: Lifecycle,
    pub atomicity: Atomicity,
    pub effect: CommandEffect,
}

impl CommandSpec {
    /// A command name has the event-type grammar: `namespace.verb`.
    pub fn validate_name(name: &str) -> Result<(), DomainError> {
        validate_type_name(name).map_err(|_| {
            DomainError::Invalid(format!(
                "command name `{name}` must be `<capability>.<verb>` in snake_case"
            ))
        })
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
}

impl Actor {
    pub fn kind(&self) -> ActorKind {
        match self {
            Actor::Human => ActorKind::Human,
            Actor::Agent { .. } => ActorKind::Agent,
            Actor::Lens { .. } => ActorKind::Lens,
            Actor::System => ActorKind::System,
        }
    }

    /// An agent, or a lens acting for one (at any depth). Agent-only
    /// rules — the agent policy, "can never confirm" — apply to both.
    pub fn is_agent_driven(&self) -> bool {
        match self {
            Actor::Agent { .. } => true,
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.is_agent_driven(),
            Actor::Human | Actor::System => false,
        }
    }

    /// The agent behind this actor, if any (see [`Self::is_agent_driven`]).
    pub fn agent_thread(&self) -> Option<Option<ThreadId>> {
        match self {
            Actor::Agent { thread_id, .. } => Some(*thread_id),
            Actor::Lens { on_behalf_of, .. } => on_behalf_of.agent_thread(),
            Actor::Human | Actor::System => None,
        }
    }

    pub fn invoker(&self) -> Invoker {
        match self {
            Actor::Human | Actor::System => Invoker::Human,
            Actor::Agent { .. } => Invoker::Agent,
            Actor::Lens { .. } => Invoker::Lens,
        }
    }

    /// The `source` an event or audit row records: `human`, `agent:thr3`,
    /// `lens:acme/blocked`, `system`.
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
        }
    }

    /// The id part of `source`, for the audit's `actor_id`.
    pub fn id(&self) -> Option<String> {
        match self {
            Actor::Agent {
                thread_id: Some(t), ..
            } => Some(t.to_string()),
            Actor::Lens { lens_id, .. } => Some(lens_id.clone()),
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
    /// The handler failed.
    Failed { message: String },
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
            CommandError::Failed { message } => write!(f, "command failed: {message}"),
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
}

impl InputValidator {
    pub fn compile(schema: &Value) -> Result<Self, DomainError> {
        let validator = jsonschema::validator_for(schema)
            .map_err(|e| DomainError::Invariant(format!("input schema: {e}")))?;
        Ok(Self { validator })
    }

    /// The first violation, as `Invalid { field: <JSON pointer>, … }`.
    pub fn check(&self, input: &Value) -> Result<(), CommandError> {
        match self.validator.iter_errors(input).next() {
            None => Ok(()),
            Some(err) => {
                let path = err.instance_path().to_string();
                Err(CommandError::Invalid {
                    field: if path.is_empty() { None } else { Some(path) },
                    message: err.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    }

    #[test]
    fn command_names_follow_the_type_grammar() {
        assert!(CommandSpec::validate_name("work_item.transition").is_ok());
        assert!(CommandSpec::validate_name("transition").is_err());
        assert!(CommandSpec::validate_name("Work.Item").is_err());
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
