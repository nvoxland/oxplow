//! What a tool call records of itself (`agent.tool.*`, then the
//! `agent_tool_call` row behind `v_tool_call` / `v_context_read` /
//! `v_struggle`): the files it names, relative to the thread's worktree,
//! and a short detail. The call is already in oxplow's vocabulary (its
//! harness mapped it, `AgentHarness::tool_use`). Pure so it's tested
//! without a hook server.

use std::path::Path;

use oxplow_domain::agent::tool::ToolUse;

/// Longest `detail` kept (a command, a search pattern, a question).
pub const MAX_DETAIL: usize = 300;

/// What a call records of its files and its detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// Worktree-relative when inside `worktree`, else as given.
    pub paths: Vec<String>,
    /// At most [`MAX_DETAIL`] characters.
    pub detail: Option<String>,
}

pub fn recorded(tool: &ToolUse, worktree: &Path) -> Recorded {
    Recorded {
        paths: tool.paths.iter().map(|p| relativize(p, worktree)).collect(),
        detail: tool
            .detail
            .as_ref()
            .map(|d| d.chars().take(MAX_DETAIL).collect()),
    }
}

/// Repo-relative when inside the worktree; the root itself is `.`.
fn relativize(path: &str, worktree: &Path) -> String {
    match Path::new(path)
        .strip_prefix(worktree)
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
    use oxplow_domain::agent::tool::ToolKind;

    fn call(paths: &[&str], detail: Option<&str>) -> ToolUse {
        ToolUse {
            name: "x".into(),
            kind: ToolKind::Read,
            paths: paths.iter().map(|p| p.to_string()).collect(),
            detail: detail.map(str::to_string),
            ..ToolUse::default()
        }
    }

    /// The worktree root itself reads as `.`, not an empty path (tsk371).
    #[test]
    fn the_worktree_root_is_dot() {
        let r = recorded(&call(&["/repo"], None), Path::new("/repo"));
        assert_eq!(r.paths, vec![".".to_string()]);
    }

    #[test]
    fn paths_are_made_worktree_relative() {
        let r = recorded(
            &call(
                &["/repo/.context/usability.md", "src/a.rs", "/elsewhere/x.md"],
                None,
            ),
            Path::new("/repo"),
        );
        assert_eq!(
            r.paths,
            vec![
                ".context/usability.md".to_string(),
                "src/a.rs".to_string(),
                "/elsewhere/x.md".to_string()
            ]
        );
    }

    #[test]
    fn a_long_detail_is_cut() {
        let long = "x".repeat(1000);
        let r = recorded(&call(&[], Some(&long)), Path::new("/repo"));
        assert_eq!(r.detail.unwrap().chars().count(), MAX_DETAIL);
    }
}
