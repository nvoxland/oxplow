//! oxplow's own view of what an ACP agent says. `wire.rs` is the only
//! module that sees SDK types; everything past it (mapping, transcript,
//! the session, the UI) speaks these, so an SDK bump touches one file.

use serde::{Deserialize, Serialize};

/// The ACP tool categories, mirrored so the SDK's enum stays in `wire.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// One file change a tool call reports. `old_text: None` means a new file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiff {
    pub path: String,
    pub old_text: Option<String>,
    pub new_text: String,
}

/// A tool call as last reported: the initial `tool_call` with every
/// `tool_call_update` for its id applied.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub title: String,
    /// The agent's programmatic tool name, when it sends one.
    pub name: Option<String>,
    pub kind: ToolKind,
    pub status: ToolStatus,
    /// Paths from `locations` (absolute, per the protocol).
    pub locations: Vec<String>,
    pub raw_input: Option<serde_json::Value>,
    pub raw_output: Option<serde_json::Value>,
    pub diffs: Vec<ToolDiff>,
    /// Text content blocks (command output, messages).
    pub text: Vec<String>,
}

/// The fields a `tool_call_update` changes; `None` leaves one as is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolCallPatch {
    pub id: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub locations: Option<Vec<String>>,
    pub raw_input: Option<serde_json::Value>,
    pub raw_output: Option<serde_json::Value>,
    pub diffs: Option<Vec<ToolDiff>>,
    pub text: Option<Vec<String>>,
}

impl ToolCall {
    pub fn apply(&mut self, p: &ToolCallPatch) {
        if let Some(v) = &p.title {
            self.title = v.clone();
        }
        if p.name.is_some() {
            self.name = p.name.clone();
        }
        if let Some(v) = p.kind {
            self.kind = v;
        }
        if let Some(v) = p.status {
            self.status = v;
        }
        if let Some(v) = &p.locations {
            self.locations = v.clone();
        }
        if p.raw_input.is_some() {
            self.raw_input = p.raw_input.clone();
        }
        if p.raw_output.is_some() {
            self.raw_output = p.raw_output.clone();
        }
        if let Some(v) = &p.diffs {
            self.diffs = v.clone();
        }
        if let Some(v) = &p.text {
            self.text = v.clone();
        }
    }

    /// A call first seen as an update (the agent may skip `tool_call`).
    pub fn from_patch(p: &ToolCallPatch) -> Self {
        let mut t = ToolCall {
            id: p.id.clone(),
            ..Default::default()
        };
        t.apply(p);
        t
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    pub status: PlanStatus,
}

/// Context-window occupancy and cumulative cost from `usage_update`. It
/// drives the context meter only — never token accounting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used: u64,
    pub size: u64,
    pub cost_amount: Option<f64>,
    pub cost_currency: Option<String>,
}

/// One `session/update`, in oxplow's terms.
#[derive(Debug, Clone, PartialEq)]
pub enum AcpUpdate {
    UserChunk(String),
    AgentChunk(String),
    ThoughtChunk(String),
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCallPatch),
    Plan(Vec<PlanEntry>),
    Usage(ContextUsage),
    /// Mode, command, config and info updates: nothing oxplow shows yet.
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: PermissionKind,
}

/// A `session/request_permission`.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionAsk {
    pub tool: ToolCallPatch,
    pub options: Vec<PermissionOption>,
}

/// How a permission request was answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionAnswer {
    Selected { option_id: String },
    Cancelled,
}
