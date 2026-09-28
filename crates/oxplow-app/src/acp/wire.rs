//! The only module that names `agent-client-protocol` schema types. It
//! converts them to and from `model.rs`, so the pinned SDK (`=2.2.0`, a
//! young 2.x) can change without touching the rest of `acp/`.

use agent_client_protocol::schema::v1 as sdk;

use super::model::{
    AcpUpdate, ContextUsage, PermissionAnswer, PermissionAsk, PermissionKind, PermissionOption,
    PlanEntry, PlanStatus, ToolCall, ToolCallPatch, ToolDiff, ToolKind, ToolStatus,
};

/// Text kept per content block; a runaway command output is cut here.
pub const MAX_TEXT: usize = 64 * 1024;

pub fn update(u: sdk::SessionUpdate) -> AcpUpdate {
    use sdk::SessionUpdate as U;
    match u {
        U::UserMessageChunk(c) => AcpUpdate::UserChunk(block_text(&c.content)),
        U::AgentMessageChunk(c) => AcpUpdate::AgentChunk(block_text(&c.content)),
        U::AgentThoughtChunk(c) => AcpUpdate::ThoughtChunk(block_text(&c.content)),
        U::ToolCall(t) => AcpUpdate::ToolCall(tool_call(t)),
        U::ToolCallUpdate(u) => AcpUpdate::ToolCallUpdate(tool_patch(u)),
        U::Plan(p) => AcpUpdate::Plan(
            p.entries
                .into_iter()
                .map(|e| PlanEntry {
                    content: e.content,
                    status: match e.status {
                        sdk::PlanEntryStatus::Completed => PlanStatus::Completed,
                        sdk::PlanEntryStatus::InProgress => PlanStatus::InProgress,
                        _ => PlanStatus::Pending,
                    },
                })
                .collect(),
        ),
        U::UsageUpdate(u) => AcpUpdate::Usage(ContextUsage {
            used: u.used,
            size: u.size,
            cost_amount: u.cost.as_ref().map(|c| c.amount),
            cost_currency: u.cost.map(|c| c.currency),
        }),
        _ => AcpUpdate::Ignored,
    }
}

pub fn tool_call(t: sdk::ToolCall) -> ToolCall {
    let (diffs, text) = contents(t.content);
    ToolCall {
        id: t.tool_call_id.0.to_string(),
        title: t.title,
        name: t.name,
        kind: kind(t.kind),
        status: status(t.status),
        locations: t.locations.iter().map(location).collect(),
        raw_input: t.raw_input,
        raw_output: t.raw_output.map(cap_value),
        diffs,
        text,
    }
}

pub fn tool_patch(u: sdk::ToolCallUpdate) -> ToolCallPatch {
    let f = u.fields;
    let (diffs, text) = match f.content {
        Some(c) => {
            let (d, t) = contents(c);
            (Some(d), Some(t))
        }
        None => (None, None),
    };
    ToolCallPatch {
        id: u.tool_call_id.0.to_string(),
        title: f.title,
        name: f.name,
        kind: f.kind.map(kind),
        status: f.status.map(status),
        locations: f.locations.map(|l| l.iter().map(location).collect()),
        raw_input: f.raw_input,
        raw_output: f.raw_output.map(cap_value),
        diffs,
        text,
    }
}

pub fn permission_ask(r: sdk::RequestPermissionRequest) -> PermissionAsk {
    PermissionAsk {
        tool: tool_patch(r.tool_call),
        options: r
            .options
            .into_iter()
            .map(|o| PermissionOption {
                id: o.option_id.0.to_string(),
                name: o.name,
                kind: match o.kind {
                    sdk::PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
                    sdk::PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
                    sdk::PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
                    _ => PermissionKind::RejectOnce,
                },
            })
            .collect(),
    }
}

pub fn permission_response(a: &PermissionAnswer) -> sdk::RequestPermissionResponse {
    let outcome = match a {
        PermissionAnswer::Selected { option_id } => sdk::RequestPermissionOutcome::Selected(
            sdk::SelectedPermissionOutcome::new(option_id.clone()),
        ),
        PermissionAnswer::Cancelled => sdk::RequestPermissionOutcome::Cancelled,
    };
    sdk::RequestPermissionResponse::new(outcome)
}

fn kind(k: sdk::ToolKind) -> ToolKind {
    use sdk::ToolKind as K;
    match k {
        K::Read => ToolKind::Read,
        K::Edit => ToolKind::Edit,
        K::Delete => ToolKind::Delete,
        K::Move => ToolKind::Move,
        K::Search => ToolKind::Search,
        K::Execute => ToolKind::Execute,
        K::Think => ToolKind::Think,
        K::Fetch => ToolKind::Fetch,
        K::SwitchMode => ToolKind::SwitchMode,
        _ => ToolKind::Other,
    }
}

fn status(s: sdk::ToolCallStatus) -> ToolStatus {
    use sdk::ToolCallStatus as S;
    match s {
        S::InProgress => ToolStatus::InProgress,
        S::Completed => ToolStatus::Completed,
        S::Failed => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn location(l: &sdk::ToolCallLocation) -> String {
    l.path.to_string_lossy().into_owned()
}

fn contents(c: Vec<sdk::ToolCallContent>) -> (Vec<ToolDiff>, Vec<String>) {
    let mut diffs = Vec::new();
    let mut text = Vec::new();
    for item in c {
        match item {
            sdk::ToolCallContent::Diff(d) => diffs.push(ToolDiff {
                path: d.path.to_string_lossy().into_owned(),
                old_text: d.old_text,
                new_text: d.new_text,
            }),
            sdk::ToolCallContent::Content(c) => text.push(block_text(&c.content)),
            // No terminal capability is advertised, so none should arrive.
            _ => {}
        }
    }
    (diffs, text)
}

/// Text for a content block; non-text blocks become a short placeholder.
fn block_text(b: &sdk::ContentBlock) -> String {
    match b {
        sdk::ContentBlock::Text(t) => cap(&t.text),
        other => {
            let v = serde_json::to_value(other).unwrap_or_default();
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("content");
            match v
                .get("uri")
                .or_else(|| v.pointer("/resource/uri"))
                .and_then(|u| u.as_str())
            {
                Some(uri) => format!("[{ty}: {uri}]"),
                None => format!("[{ty}]"),
            }
        }
    }
}

fn cap(s: &str) -> String {
    if s.len() <= MAX_TEXT {
        return s.to_string();
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated]", &s[..end])
}

fn cap_value(v: serde_json::Value) -> serde_json::Value {
    match &v {
        serde_json::Value::String(s) if s.len() > MAX_TEXT => serde_json::Value::String(cap(s)),
        serde_json::Value::String(_) => v,
        _ if v.to_string().len() > MAX_TEXT => serde_json::json!({ "truncated": true }),
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upd(v: serde_json::Value) -> AcpUpdate {
        update(serde_json::from_value(v).unwrap())
    }

    #[test]
    fn chunks_become_text() {
        assert_eq!(
            upd(
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}})
            ),
            AcpUpdate::AgentChunk("hi".into())
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hm"}})
            ),
            AcpUpdate::ThoughtChunk("hm".into())
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "user_message_chunk", "content": {"type": "resource_link", "uri": "file:///a.rs", "name": "a.rs"}})
            ),
            AcpUpdate::UserChunk("[resource_link: file:///a.rs]".into())
        );
    }

    #[test]
    fn tool_call_carries_diffs_locations_and_input() {
        let AcpUpdate::ToolCall(t) = upd(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "c1",
            "title": "Edit a.rs",
            "kind": "edit",
            "status": "pending",
            "locations": [{"path": "/w/a.rs", "line": 3}],
            "rawInput": {"file_path": "/w/a.rs"},
            "content": [
                {"type": "diff", "path": "/w/a.rs", "oldText": "x", "newText": "y"},
                {"type": "content", "content": {"type": "text", "text": "note"}}
            ]
        })) else {
            panic!("not a tool call")
        };
        assert_eq!(t.id, "c1");
        assert_eq!(t.kind, ToolKind::Edit);
        assert_eq!(t.status, ToolStatus::Pending);
        assert_eq!(t.locations, vec!["/w/a.rs".to_string()]);
        assert_eq!(t.raw_input, Some(json!({"file_path": "/w/a.rs"})));
        assert_eq!(
            t.diffs,
            vec![ToolDiff {
                path: "/w/a.rs".into(),
                old_text: Some("x".into()),
                new_text: "y".into()
            }]
        );
        assert_eq!(t.text, vec!["note".to_string()]);
    }

    #[test]
    fn tool_call_update_is_a_sparse_patch() {
        let AcpUpdate::ToolCallUpdate(p) = upd(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "status": "completed",
            "rawOutput": {"exit_code": 0}
        })) else {
            panic!("not an update")
        };
        assert_eq!(p.id, "c1");
        assert_eq!(p.status, Some(ToolStatus::Completed));
        assert_eq!(p.raw_output, Some(json!({"exit_code": 0})));
        assert_eq!(p.title, None);
        assert_eq!(p.diffs, None);
    }

    #[test]
    fn plan_and_usage() {
        assert_eq!(
            upd(json!({"sessionUpdate": "plan", "entries": [
                {"content": "a", "priority": "high", "status": "completed"},
                {"content": "b", "priority": "low", "status": "pending"}
            ]})),
            AcpUpdate::Plan(vec![
                PlanEntry {
                    content: "a".into(),
                    status: PlanStatus::Completed
                },
                PlanEntry {
                    content: "b".into(),
                    status: PlanStatus::Pending
                },
            ])
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "usage_update", "used": 10, "size": 100, "cost": {"amount": 0.5, "currency": "USD"}})
            ),
            AcpUpdate::Usage(ContextUsage {
                used: 10,
                size: 100,
                cost_amount: Some(0.5),
                cost_currency: Some("USD".into())
            })
        );
        assert_eq!(
            upd(json!({"sessionUpdate": "current_mode_update", "currentModeId": "x"})),
            AcpUpdate::Ignored
        );
    }

    #[test]
    fn long_text_is_capped() {
        let big = "é".repeat(MAX_TEXT);
        let AcpUpdate::AgentChunk(t) = upd(
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": big}}),
        ) else {
            panic!()
        };
        assert!(t.len() < MAX_TEXT + 32);
        assert!(t.ends_with("[truncated]"));
    }

    #[test]
    fn permission_round_trip() {
        let ask = permission_ask(
            serde_json::from_value(json!({
                "sessionId": "s1",
                "toolCall": {"toolCallId": "c1", "title": "Write b.rs", "kind": "edit"},
                "options": [
                    {"optionId": "a", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "r", "name": "Reject", "kind": "reject_once"}
                ]
            }))
            .unwrap(),
        );
        assert_eq!(ask.tool.id, "c1");
        assert_eq!(ask.tool.kind, Some(ToolKind::Edit));
        assert_eq!(ask.options[1].kind, PermissionKind::RejectOnce);

        let sel = serde_json::to_value(permission_response(&PermissionAnswer::Selected {
            option_id: "r".into(),
        }))
        .unwrap();
        assert_eq!(
            sel,
            json!({"outcome": {"outcome": "selected", "optionId": "r"}})
        );
        let cancel =
            serde_json::to_value(permission_response(&PermissionAnswer::Cancelled)).unwrap();
        assert_eq!(cancel, json!({"outcome": {"outcome": "cancelled"}}));
    }
}
