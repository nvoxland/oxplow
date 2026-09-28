//! Turn a PostToolUse hook payload into a persisted tool-call row
//! (`agent_tool_call`), for `v_tool_call` / `v_context_read` /
//! `v_struggle`. Pure so it's tested without a hook server.

use std::path::Path;

/// Longest `detail` kept (a Bash command, a search pattern, a question).
pub const MAX_DETAIL: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallParts {
    pub tool: String,
    /// Repo-relative when inside `project_dir`, else as given.
    pub path: Option<String>,
    pub detail: Option<String>,
    /// `Some(false)` on a reported failure, `None` when unknown.
    pub ok: Option<bool>,
}

pub fn parse_tool_call(payload_json: &str, project_dir: &Path) -> Option<ToolCallParts> {
    let v: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let tool = v.get("tool_name")?.as_str()?.to_string();
    let input = v.get("tool_input");
    let str_field = |k: &str| input.and_then(|i| i.get(k)).and_then(|x| x.as_str());

    let path = ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|k| str_field(k))
        .map(|p| relativize(p, project_dir));

    let detail = str_field("command")
        .or_else(|| str_field("pattern"))
        .or_else(|| str_field("query"))
        .or_else(|| str_field("url"))
        // `await_user`: the question the agent is waiting on (read by the
        // oxplow-review Waiting on Me lens).
        .or_else(|| str_field("question"))
        .map(|d| d.chars().take(MAX_DETAIL).collect());

    let response = v.get("tool_response");
    let reported_error = response
        .and_then(|r| r.get("is_error").or_else(|| r.get("isError")))
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    let ok = if reported_error {
        Some(false)
    } else if tool == "Bash" {
        crate::collection::parse_bash_post_tool(payload_json)
            .and_then(|b| b.exit_code)
            .map(|c| c == 0)
    } else {
        Some(true)
    };
    Some(ToolCallParts {
        tool,
        path,
        detail,
        ok,
    })
}

/// Repo-relative when inside the project; the root itself is `.`.
fn relativize(path: &str, project_dir: &Path) -> String {
    match Path::new(path)
        .strip_prefix(project_dir)
        .ok()
        .and_then(|p| p.to_str())
    {
        Some("") => ".".to_string(),
        Some(rel) => rel.to_string(),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Option<ToolCallParts> {
        parse_tool_call(&v.to_string(), Path::new("/repo"))
    }

    /// The project root itself reads as `.`, not an empty path (tsk371).
    #[test]
    fn the_project_root_is_dot() {
        let p =
            parse(json!({"tool_name": "Grep", "tool_input": {"pattern": "x", "path": "/repo"}}))
                .unwrap();
        assert_eq!(p.path.as_deref(), Some("."));
    }

    #[test]
    fn paths_are_made_repo_relative() {
        let p = parse(json!({"tool_name": "Read", "tool_input": {"file_path": "/repo/.context/usability.md"}})).unwrap();
        assert_eq!(p.tool, "Read");
        assert_eq!(p.path.as_deref(), Some(".context/usability.md"));
        assert_eq!(p.ok, Some(true));
        let p =
            parse(json!({"tool_name": "Edit", "tool_input": {"file_path": "src/a.rs"}})).unwrap();
        assert_eq!(p.path.as_deref(), Some("src/a.rs"));
        let p = parse(json!({"tool_name": "Read", "tool_input": {"file_path": "/elsewhere/x.md"}}))
            .unwrap();
        assert_eq!(p.path.as_deref(), Some("/elsewhere/x.md"));
        let p = parse(
            json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "/repo/n.ipynb"}}),
        )
        .unwrap();
        assert_eq!(p.path.as_deref(), Some("n.ipynb"));
    }

    #[test]
    fn bash_keeps_the_command_and_exit_status() {
        let p = parse(json!({"tool_name": "Bash", "tool_input": {"command": "cargo test"}, "tool_response": {"exit_code": 101}})).unwrap();
        assert_eq!(p.detail.as_deref(), Some("cargo test"));
        assert_eq!(p.ok, Some(false));
        let p = parse(json!({"tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": {"stdout": "x"}})).unwrap();
        assert_eq!(p.ok, None, "no exit code → unknown");
        let long = "x".repeat(1000);
        let p = parse(json!({"tool_name": "Bash", "tool_input": {"command": long}})).unwrap();
        assert_eq!(p.detail.unwrap().chars().count(), MAX_DETAIL);
    }

    #[test]
    fn search_patterns_and_errors() {
        let p = parse(
            json!({"tool_name": "Grep", "tool_input": {"pattern": "fn main", "path": "/repo/src"}}),
        )
        .unwrap();
        assert_eq!(p.detail.as_deref(), Some("fn main"));
        assert_eq!(p.path.as_deref(), Some("src"));
        let p = parse(json!({"tool_name": "Edit", "tool_input": {"file_path": "a"}, "tool_response": {"is_error": true}})).unwrap();
        assert_eq!(p.ok, Some(false));
        assert_eq!(parse(json!({"tool_input": {}})), None);
        assert_eq!(parse_tool_call("not json", Path::new("/repo")), None);
    }

    #[test]
    fn an_await_user_call_keeps_its_question() {
        let p = parse(json!({
            "tool_name": "mcp__oxplow__await_user",
            "tool_input": {"threadId": "thr1", "question": "Pick A or B?"}
        }))
        .unwrap();
        assert_eq!(p.detail.as_deref(), Some("Pick A or B?"));
    }
}
