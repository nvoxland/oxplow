//! The meta-model: what host and provider say to each other, defined once
//! as Rust types (serde + schemars). The schema goldens under `schemas/`
//! are generated from these; every message on the wire validates against
//! them (`validate`).
//!
//! The methods, host → provider:
//!
//! - `initialize` — [`InitializeParams`] → [`InitializeResult`]: the
//!   provider says what it is and declares what it offers (capabilities,
//!   commands, event types, collectors, its config's schema). The host
//!   compares the declarations with the checked-in ones it approved.
//! - `check` — [`CheckParams`] → [`CheckResult`]: validate an instance's
//!   config (and which credentials are present); a clean check returns an
//!   opaque [`Handle`] the other calls carry.
//! - `discover` — [`DiscoverParams`] → [`DiscoverResult`]: the entities
//!   the configured instance can read.
//! - `invoke` — [`InvokeParams`] → [`InvokeResult`]: run one declared
//!   command; the result, the events it produced, and its inverse.
//! - `read` — [`ReadParams`] → [`ReadResult`]: run a collector, streaming
//!   rows as `$/record` and checkpoints as `$/state` before the result.
//! - `shutdown` — no params → `null`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version this crate speaks.
pub const PROTOCOL_VERSION: &str = "2";

pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const CHECK: &str = "check";
    pub const DISCOVER: &str = "discover";
    pub const INVOKE: &str = "invoke";
    pub const READ: &str = "read";
    pub const SHUTDOWN: &str = "shutdown";
}

/// Who is speaking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Party {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitializeParams {
    pub protocol_version: String,
    pub host: Party,
}

/// A capability the provider implements (`work_items`), with the
/// capability's own feature flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDecl {
    pub capability: String,
    #[serde(default)]
    pub features: Value,
}

/// A command the provider runs, registered on the host's bus as
/// `<provider>.<name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    pub name: String,
    pub summary: String,
    /// JSON Schema of the input.
    pub input_schema: Value,
    /// `never`, `always` or `destructive`.
    pub confirm: String,
    /// `write`, `read` or `record`.
    pub effect: String,
    pub undoable: bool,
}

/// An event type the provider's commands produce, with its payload's
/// JSON Schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventTypeDecl {
    #[serde(rename = "type")]
    pub event_type: String,
    pub v: u32,
    pub schema: Value,
}

/// A collector: a read that produces rows of an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectorDecl {
    pub name: String,
    pub entity: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub provider: Party,
    pub capabilities: Vec<CapabilityDecl>,
    pub commands: Vec<CommandDecl>,
    pub event_types: Vec<EventTypeDecl>,
    pub collectors: Vec<CollectorDecl>,
    /// JSON Schema of an instance's config.
    pub config_schema: Value,
}

/// An opaque token for a checked instance, carried by the other calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Handle(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    pub config: Value,
    /// The names of the credentials the host holds for the instance (the
    /// values stay in the keychain; the provider's environment has them).
    pub credentials: Vec<String>,
}

/// One thing wrong with a config: where, and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    /// A JSON pointer into the config (`/team`), or `""` for the whole.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub problems: Vec<Problem>,
    /// Present exactly when there are no problems.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<Handle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverParams {
    pub handle: Handle,
}

/// An entity a configured instance can read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityDecl {
    pub name: String,
    pub description: String,
    /// JSON Schema of a row.
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverResult {
    pub entities: Vec<EntityDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvokeParams {
    pub handle: Handle,
    /// A declared command's name.
    pub command: String,
    pub input: Value,
    /// The write's idempotency key, when the host may send it again (a
    /// reply lost, a call cut off). A provider that declares
    /// `idempotent_writes` does a write sent twice with one key once and
    /// answers the second as the first; a key sent with another write is
    /// `InvalidInput` at `/idempotency_key`. One that doesn't declare it
    /// may ignore the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// An event a command produced, for the host to log (its type is one
/// the provider declared).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventDraft {
    #[serde(rename = "type")]
    pub event_type: String,
    pub v: u32,
    pub payload: Value,
    /// Refs the event is about.
    #[serde(default)]
    pub subject: Vec<String>,
}

/// A command call (an inverse).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandCall {
    pub command: String,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvokeResult {
    pub result: Value,
    #[serde(default)]
    pub events: Vec<EventDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<CommandCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadParams {
    pub handle: Handle,
    /// A declared collector's name.
    pub collector: String,
    /// The last `$/state` checkpoint, to resume from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadResult {
    /// How many `$/record`s the read streamed.
    pub records: u64,
}
