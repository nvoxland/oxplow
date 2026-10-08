//! ACP tool calls in oxplow's vocabulary (`oxplow_domain::agent::tool`):
//! the protocol's own tool kinds map onto oxplow's, so the policy gate and
//! the recorders read an ACP agent's calls the way they read any harness's.
//! Pure.
//!
//! Edit, delete and move are edits. Their paths come from `locations`, the
//! diffs, and the path-like keys adapters put in `rawInput`, so an agent
//! that fills only one of them is still covered.

use oxplow_domain::agent::tool::{ToolKind as Kind, ToolUse};

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

/// Whether an ACP call writes files (edit, delete, move).
pub fn is_write(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
}

/// The call in oxplow's vocabulary; `None` for calls oxplow doesn't record
/// (thinking, mode switches).
pub fn tool_use(t: &ToolCall) -> Option<ToolUse> {
    let mcp = mcp_name(t);
    let kind = match t.kind {
        ToolKind::Think | ToolKind::SwitchMode => return None,
        _ if mcp.is_some() => Kind::Mcp,
        ToolKind::Read => Kind::Read,
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => Kind::Edit,
        ToolKind::Search => Kind::Search,
        ToolKind::Execute => Kind::Shell,
        ToolKind::Fetch => Kind::Fetch,
        ToolKind::Other => Kind::Other,
    };
    let input = |keys: &[&str]| {
        t.raw_input.as_ref().and_then(|v| {
            keys.iter()
                .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
                .map(str::to_string)
        })
    };
    let command = (kind == Kind::Shell)
        .then(|| input(&["command", "cmd"]).unwrap_or_else(|| t.title.clone()));
    let detail = match kind {
        Kind::Shell => command.clone(),
        Kind::Search => input(&["pattern", "query", "regex"]),
        Kind::Fetch => input(&["url", "uri"]),
        _ => None,
    }
    .or_else(|| Some(t.title.clone()).filter(|s| !s.is_empty()));
    let (ok, exit_code) = match t.status {
        ToolStatus::Completed | ToolStatus::Failed => (
            Some(t.status == ToolStatus::Completed),
            t.raw_output.as_ref().and_then(|o| {
                ["exit_code", "exitCode", "code"]
                    .iter()
                    .find_map(|k| o.get(*k).and_then(|x| x.as_i64()))
            }),
        ),
        _ => (None, None),
    };
    Some(ToolUse {
        name: label(t),
        kind,
        paths: paths(t),
        command,
        detail,
        call_id: Some(t.id.clone()).filter(|id| !id.is_empty()),
        ok,
        exit_code,
        question: None,
    })
}

/// What the call is called where a person reads it: its MCP tool, `Delete`
/// / `Move`, else the agent's name or title for it.
pub fn label(t: &ToolCall) -> String {
    if let Some(mcp) = mcp_name(t) {
        return mcp;
    }
    match t.kind {
        ToolKind::Delete => "Delete".to_string(),
        ToolKind::Move => "Move".to_string(),
        _ => t
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| t.title.clone()),
    }
}

/// What the ingest stores of the call by hash: its input as the agent sent
/// it, and once it finished, its output with whether it failed.
pub fn content(t: &ToolCall) -> serde_json::Value {
    let mut v = serde_json::json!({
        "tool_input": t.raw_input.clone().unwrap_or_else(|| serde_json::json!({})),
    });
    if let Some(r) = tool_response(t) {
        v["tool_response"] = r;
    }
    v
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

fn mcp_name(t: &ToolCall) -> Option<String> {
    [t.name.as_deref(), Some(t.title.as_str())]
        .into_iter()
        .flatten()
        .filter_map(|s| s.split_whitespace().next())
        .find(|s| s.starts_with("mcp__"))
        .map(str::to_string)
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
    fn writes_are_edits_of_every_path() {
        let mut t = call(ToolKind::Edit);
        t.locations = vec!["/w/a.rs".into()];
        t.diffs = vec![diff("/w/b.rs", Some("o"))];
        t.raw_input = Some(json!({"file_path": "/w/a.rs", "notebook_path": "/w/n.ipynb"}));
        let u = tool_use(&t).unwrap();
        assert_eq!(u.kind, Kind::Edit);
        assert_eq!(u.paths, vec!["/w/a.rs", "/w/b.rs", "/w/n.ipynb"]);
        assert_eq!(u.call_id.as_deref(), Some("c1"));

        let mut mv = call(ToolKind::Move);
        mv.raw_input = Some(json!({"source": "/w/a", "destination": "/w/b"}));
        let u = tool_use(&mv).unwrap();
        assert_eq!((u.kind, u.name.as_str()), (Kind::Edit, "Move"));
        assert_eq!(u.paths, vec!["/w/a", "/w/b"]);
        assert_eq!(tool_use(&call(ToolKind::Delete)).unwrap().name, "Delete");
    }

    #[test]
    fn the_protocols_kinds_map_onto_oxplows() {
        for (k, want) in [
            (ToolKind::Read, Kind::Read),
            (ToolKind::Search, Kind::Search),
            (ToolKind::Execute, Kind::Shell),
            (ToolKind::Fetch, Kind::Fetch),
            (ToolKind::Other, Kind::Other),
        ] {
            assert_eq!(tool_use(&call(k)).unwrap().kind, want, "{k:?}");
        }
        assert_eq!(tool_use(&call(ToolKind::Think)), None);
        assert_eq!(tool_use(&call(ToolKind::SwitchMode)), None);
        let mut mcp = call(ToolKind::Other);
        mcp.title = "mcp__oxplow__list_work_items (MCP)".into();
        let u = tool_use(&mcp).unwrap();
        assert_eq!(
            (u.kind, u.name.as_str()),
            (Kind::Mcp, "mcp__oxplow__list_work_items")
        );
        let mut named = call(ToolKind::Other);
        named.name = Some("custom".into());
        assert_eq!(tool_use(&named).unwrap().name, "custom");
    }

    #[test]
    fn a_shell_call_keeps_its_command_and_exit_code() {
        let mut t = call(ToolKind::Execute);
        t.title = "cargo test".into();
        t.status = ToolStatus::Failed;
        t.raw_output = Some(json!({"exit_code": 101}));
        let u = tool_use(&t).unwrap();
        assert_eq!(u.command.as_deref(), Some("cargo test"));
        assert_eq!((u.ok, u.exit_code), (Some(false), Some(101)));
        assert_eq!(
            content(&t),
            json!({"tool_input": {}, "tool_response": {"exit_code": 101, "is_error": true}})
        );
        let mut given = call(ToolKind::Execute);
        given.raw_input = Some(json!({"command": "ls"}));
        given.raw_output = Some(json!("out"));
        given.status = ToolStatus::Completed;
        let u = tool_use(&given).unwrap();
        assert_eq!(u.command.as_deref(), Some("ls"));
        assert_eq!((u.ok, u.exit_code), (Some(true), None));
        assert_eq!(
            content(&given)["tool_response"],
            json!({"output": "out", "is_error": false})
        );
    }

    #[test]
    fn a_running_call_has_no_outcome_and_a_search_its_pattern() {
        let mut r = call(ToolKind::Read);
        r.locations = vec!["/w/a.rs".into()];
        let u = tool_use(&r).unwrap();
        assert_eq!(u.ok, None);
        assert!(content(&r).get("tool_response").is_none());
        let mut g = call(ToolKind::Search);
        g.raw_input = Some(json!({"query": "fn main"}));
        assert_eq!(tool_use(&g).unwrap().detail.as_deref(), Some("fn main"));
    }
}
