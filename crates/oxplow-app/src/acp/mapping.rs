//! ACP tool calls in oxplow's terms: the policy intent the gate checks and
//! the canonical (Claude-shaped) events `AgentContext` records. Pure.
//!
//! Edit, delete and move are worktree writes. Their paths come from
//! `locations`, the diffs, and the path-like keys adapters put in
//! `rawInput`, so an agent that fills only one of them is still covered.

use oxplow_runtime::policy::{IntentKind, ToolIntent};

use super::model::{ToolCall, ToolKind, ToolStatus};

/// `rawInput` keys adapters use for the file a tool touches.
const PATH_KEYS: &[&str] = &[
    "file_path",
    "path",
    "absolute_path",
    "notebook_path",
    "source",
    "destination",
    "old_path",
    "new_path",
];

/// An owned [`ToolIntent`].
#[derive(Debug, Clone, PartialEq)]
pub struct AcpIntent {
    pub label: String,
    pub kind: IntentKind,
    pub paths: Vec<String>,
}

impl AcpIntent {
    pub fn as_intent(&self) -> ToolIntent<'_> {
        ToolIntent {
            label: &self.label,
            kind: self.kind,
            paths: &self.paths,
        }
    }
}

pub fn is_write(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
}

pub fn intent_for(t: &ToolCall) -> AcpIntent {
    let label = match t.kind {
        ToolKind::Delete => "Delete".to_string(),
        ToolKind::Move => "Move".to_string(),
        _ => canonical_name(t).unwrap_or_else(|| display_name(t)),
    };
    AcpIntent {
        label,
        kind: if is_write(t.kind) {
            IntentKind::WorktreeWrite
        } else {
            IntentKind::Other
        },
        paths: paths(t),
    }
}

/// Every path the call names, in first-seen order.
pub fn paths(t: &ToolCall) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |p: &str| {
        if !p.is_empty() && !out.iter().any(|q| q == p) {
            out.push(p.to_string());
        }
    };
    for l in &t.locations {
        add(l);
    }
    for d in &t.diffs {
        add(&d.path);
    }
    if let Some(obj) = t.raw_input.as_ref().and_then(|v| v.as_object()) {
        for k in PATH_KEYS {
            if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
                add(s);
            }
        }
    }
    out
}

/// The Claude tool name recorders key on, or `None` for calls oxplow
/// doesn't record (thinking, mode switches).
pub fn canonical_name(t: &ToolCall) -> Option<String> {
    if let Some(mcp) = mcp_name(t) {
        return Some(mcp);
    }
    Some(
        match t.kind {
            ToolKind::Think | ToolKind::SwitchMode => return None,
            ToolKind::Read => "Read",
            ToolKind::Edit => {
                if !t.diffs.is_empty() && t.diffs.iter().all(|d| d.old_text.is_none()) {
                    "Write"
                } else {
                    "Edit"
                }
            }
            // Recorded as edits of each path so effort claims see them.
            ToolKind::Delete | ToolKind::Move => "Edit",
            ToolKind::Search => "Grep",
            ToolKind::Execute => "Bash",
            ToolKind::Fetch => "WebFetch",
            ToolKind::Other => return Some(display_name(t)),
        }
        .to_string(),
    )
}

fn mcp_name(t: &ToolCall) -> Option<String> {
    [t.name.as_deref(), Some(t.title.as_str())]
        .into_iter()
        .flatten()
        .filter_map(|s| s.split_whitespace().next())
        .find(|s| s.starts_with("mcp__"))
        .map(str::to_string)
}

fn display_name(t: &ToolCall) -> String {
    t.name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| t.title.clone())
}

/// A tool call in the canonical (Claude-shaped) vocabulary, for transports
/// whose agents don't speak it natively. [`Self::to_payload`] is the one
/// place that shape is built.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalToolEvent {
    /// `Edit`, `Write`, `Read`, `Grep`, `Bash`, `WebFetch`, `mcp__…`, …
    pub tool_name: String,
    /// `{file_path}`, `{command}`, `{pattern}`, `{url}`, …
    pub tool_input: serde_json::Value,
    /// `{is_error: bool, …}` once the call finished.
    pub tool_response: Option<serde_json::Value>,
    pub session_id: Option<String>,
}

impl CanonicalToolEvent {
    /// The hook-payload shape every recorder reads.
    pub fn to_payload(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
            "tool_name": self.tool_name,
            "tool_input": self.tool_input,
        });
        if let Some(r) = &self.tool_response {
            v["tool_response"] = r.clone();
        }
        if let Some(s) = &self.session_id {
            v["session_id"] = serde_json::Value::String(s.clone());
        }
        v
    }
}

/// The canonical events for a call: one per path for writes (recorders
/// read a single `file_path`), one otherwise, none for unrecorded kinds.
pub fn canonical_events(t: &ToolCall, session_id: Option<&str>) -> Vec<CanonicalToolEvent> {
    let Some(tool_name) = canonical_name(t) else {
        return Vec::new();
    };
    let base = match &t.raw_input {
        Some(serde_json::Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    let response = tool_response(t);
    let event = |input: serde_json::Map<String, serde_json::Value>| CanonicalToolEvent {
        tool_name: tool_name.clone(),
        tool_input: serde_json::Value::Object(input),
        tool_response: response.clone(),
        session_id: session_id.map(str::to_string),
    };
    let fill = |mut m: serde_json::Map<String, serde_json::Value>, key: &str, alts: &[&str]| {
        if !m.contains_key(key) {
            let v = alts
                .iter()
                .find_map(|k| m.get(*k).and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_else(|| t.title.clone());
            m.insert(key.into(), v.into());
        }
        m
    };

    if is_write(t.kind) && mcp_name(t).is_none() {
        return paths(t)
            .into_iter()
            .map(|p| {
                let mut m = base.clone();
                m.insert("file_path".into(), p.into());
                event(m)
            })
            .collect();
    }
    let input = match tool_name.as_str() {
        "Read" => {
            let mut m = base;
            if let Some(p) = paths(t).into_iter().next() {
                m.entry("file_path").or_insert(p.into());
            }
            m
        }
        "Bash" => fill(base, "command", &["cmd"]),
        "Grep" => fill(base, "pattern", &["query", "regex"]),
        "WebFetch" => fill(base, "url", &["uri"]),
        _ => base,
    };
    vec![event(input)]
}

/// `{is_error, …raw_output}` once the call finished; `None` while running.
fn tool_response(t: &ToolCall) -> Option<serde_json::Value> {
    let is_error = match t.status {
        ToolStatus::Completed => false,
        ToolStatus::Failed => true,
        _ => return None,
    };
    let mut m = match &t.raw_output {
        Some(serde_json::Value::Object(m)) => m.clone(),
        Some(other) => {
            let mut m = serde_json::Map::new();
            m.insert("output".into(), other.clone());
            m
        }
        None => serde_json::Map::new(),
    };
    m.insert("is_error".into(), is_error.into());
    Some(serde_json::Value::Object(m))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_canonical_event_renders_the_hook_payload_shape() {
        let ev = CanonicalToolEvent {
            tool_name: "Edit".into(),
            tool_input: serde_json::json!({"file_path": "src/a.rs"}),
            tool_response: Some(serde_json::json!({"is_error": false})),
            session_id: Some("s1".into()),
        };
        assert_eq!(
            ev.to_payload(),
            serde_json::json!({"tool_name": "Edit", "tool_input": {"file_path": "src/a.rs"}, "tool_response": {"is_error": false}, "session_id": "s1"})
        );
        // What the ingest reads: the tool, its path and outcome.
        let body = ev.to_payload();
        let parts =
            crate::tool_calls::parse_tool_call(&body.to_string(), std::path::Path::new("/p"))
                .unwrap();
        assert_eq!(
            (parts.tool.as_str(), parts.path.as_deref()),
            ("Edit", Some("src/a.rs"))
        );
    }

    use super::*;
    use crate::acp::model::ToolDiff;
    use serde_json::json;

    fn call(kind: ToolKind) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            title: "t".into(),
            kind,
            ..Default::default()
        }
    }

    fn diff(path: &str, old: Option<&str>) -> ToolDiff {
        ToolDiff {
            path: path.into(),
            old_text: old.map(str::to_string),
            new_text: "n".into(),
        }
    }

    #[test]
    fn writes_are_worktree_writes_with_every_path() {
        let mut t = call(ToolKind::Edit);
        t.locations = vec!["/w/a.rs".into()];
        t.diffs = vec![diff("/w/b.rs", Some("o"))];
        t.raw_input = Some(json!({"file_path": "/w/a.rs", "notebook_path": "/w/n.ipynb"}));
        let i = intent_for(&t);
        assert_eq!(i.kind, IntentKind::WorktreeWrite);
        assert_eq!(i.label, "Edit");
        assert_eq!(i.paths, vec!["/w/a.rs", "/w/b.rs", "/w/n.ipynb"]);

        let mut mv = call(ToolKind::Move);
        mv.raw_input = Some(json!({"source": "/w/a", "destination": "/w/b"}));
        let i = intent_for(&mv);
        assert_eq!(i.kind, IntentKind::WorktreeWrite);
        assert_eq!(i.label, "Move");
        assert_eq!(i.paths, vec!["/w/a", "/w/b"]);

        assert_eq!(intent_for(&call(ToolKind::Delete)).label, "Delete");
        assert_eq!(
            intent_for(&call(ToolKind::Delete)).kind,
            IntentKind::WorktreeWrite
        );
    }

    #[test]
    fn non_writes_are_other() {
        for k in [
            ToolKind::Read,
            ToolKind::Execute,
            ToolKind::Search,
            ToolKind::Fetch,
            ToolKind::Other,
        ] {
            assert_eq!(intent_for(&call(k)).kind, IntentKind::Other, "{k:?}");
        }
        assert_eq!(intent_for(&call(ToolKind::Execute)).label, "Bash");
    }

    #[test]
    fn canonical_names() {
        assert_eq!(
            canonical_name(&call(ToolKind::Read)).as_deref(),
            Some("Read")
        );
        assert_eq!(
            canonical_name(&call(ToolKind::Edit)).as_deref(),
            Some("Edit")
        );
        let mut w = call(ToolKind::Edit);
        w.diffs = vec![diff("/w/new.rs", None)];
        assert_eq!(canonical_name(&w).as_deref(), Some("Write"));
        assert_eq!(
            canonical_name(&call(ToolKind::Search)).as_deref(),
            Some("Grep")
        );
        assert_eq!(
            canonical_name(&call(ToolKind::Execute)).as_deref(),
            Some("Bash")
        );
        assert_eq!(
            canonical_name(&call(ToolKind::Fetch)).as_deref(),
            Some("WebFetch")
        );
        assert_eq!(canonical_name(&call(ToolKind::Think)), None);
        assert_eq!(canonical_name(&call(ToolKind::SwitchMode)), None);
        let mut mcp = call(ToolKind::Other);
        mcp.title = "mcp__oxplow__list_work_items (MCP)".into();
        assert_eq!(
            canonical_name(&mcp).as_deref(),
            Some("mcp__oxplow__list_work_items")
        );
        let mut named = call(ToolKind::Other);
        named.name = Some("custom".into());
        assert_eq!(canonical_name(&named).as_deref(), Some("custom"));
    }

    #[test]
    fn write_events_one_per_path_with_file_path() {
        let mut t = call(ToolKind::Move);
        t.status = ToolStatus::Completed;
        t.raw_input = Some(json!({"source": "/w/a", "destination": "/w/b"}));
        let ev = canonical_events(&t, Some("s1"));
        assert_eq!(ev.len(), 2);
        assert_eq!(
            ev[0].to_payload(),
            json!({
                "tool_name": "Edit",
                "tool_input": {"source": "/w/a", "destination": "/w/b", "file_path": "/w/a"},
                "tool_response": {"is_error": false},
                "session_id": "s1"
            })
        );
        assert_eq!(ev[1].tool_input["file_path"], "/w/b");
    }

    #[test]
    fn bash_event_keeps_exit_code_and_fills_command() {
        let mut t = call(ToolKind::Execute);
        t.title = "cargo test".into();
        t.status = ToolStatus::Failed;
        t.raw_output = Some(json!({"exit_code": 101}));
        let ev = canonical_events(&t, None);
        assert_eq!(
            ev[0].to_payload(),
            json!({
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "tool_response": {"exit_code": 101, "is_error": true}
            })
        );
        let mut given = call(ToolKind::Execute);
        given.raw_input = Some(json!({"command": "ls"}));
        given.raw_output = Some(json!("out"));
        given.status = ToolStatus::Completed;
        let ev = canonical_events(&given, None);
        assert_eq!(ev[0].tool_input, json!({"command": "ls"}));
        assert_eq!(
            ev[0].tool_response,
            Some(json!({"output": "out", "is_error": false}))
        );
    }

    #[test]
    fn running_calls_have_no_response_and_think_has_no_event() {
        let mut r = call(ToolKind::Read);
        r.locations = vec!["/w/a.rs".into()];
        let ev = canonical_events(&r, None);
        assert_eq!(ev[0].tool_input, json!({"file_path": "/w/a.rs"}));
        assert_eq!(ev[0].tool_response, None);
        assert!(canonical_events(&call(ToolKind::Think), None).is_empty());
        let mut g = call(ToolKind::Search);
        g.raw_input = Some(json!({"query": "fn main"}));
        assert_eq!(
            canonical_events(&g, None)[0].tool_input["pattern"],
            "fn main"
        );
    }
}
