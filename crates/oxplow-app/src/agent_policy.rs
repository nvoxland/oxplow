//! The policy every agent transport enforces: the write guard (isolation
//! only), with the I/O it needs (the thread, the stream worktrees).
//! Decisions are transport-neutral: the hook route renders them as
//! Claude's JSON, and the ACP client as permission answers. The pure rules
//! live in `oxplow_runtime::policy`. Neither an edit nor a turn's end ever
//! waits on tracked work (`.context/work-tracking.md`). See
//! `.context/agent-model.md`.

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{decide_tool, IntentKind, PolicyDecision, PolicyFacts, ToolIntent};

use crate::Services;

/// The shared agent policy (`Services.agent_policy`).
#[derive(Default)]
pub struct AgentPolicy;

/// A Claude-shaped tool call (the hook payload, or an opencode call its
/// bridge mapped onto Claude's names) as a policy intent.
#[derive(Debug, Clone)]
pub struct ClaudeIntent {
    pub label: String,
    pub kind: IntentKind,
    pub paths: Vec<String>,
}

/// Map a Claude-shaped `{tool_name, tool_input}` to an intent. `None` for a
/// tool neither rule can refuse (Read, Grep, Bash, MCP, Task, …), so
/// callers skip the policy's I/O entirely.
pub fn claude_intent(body: &serde_json::Value) -> Option<ClaudeIntent> {
    use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    if !WORKTREE_MUTATING_TOOLS.contains(&tool_name) {
        return None;
    }
    // The first key holding a string: a null `file_path` mustn't hide a
    // `notebook_path` (tsk371).
    let path = body.get("tool_input").and_then(|t| {
        ["file_path", "notebook_path", "path"]
            .iter()
            .find_map(|k| t.get(k).and_then(|v| v.as_str()))
    });
    Some(ClaudeIntent {
        label: tool_name.to_string(),
        kind: IntentKind::WorktreeWrite,
        paths: path.map(|p| vec![p.to_string()]).unwrap_or_default(),
    })
}

impl AgentPolicy {
    /// May an agent run `spec`? The bus has already admitted the invoker;
    /// this is the agent-specific layer on top: a `Write` command needs a
    /// thread that may write (`may_write`, from the bus's write gate —
    /// a queued or closed thread can read, not change). `None` means the
    /// gate had nothing to say (a read, or no thread/gate).
    pub fn check_command(
        &self,
        thread_id: Option<&ThreadId>,
        spec: &oxplow_domain::CommandSpec,
        may_write: Option<bool>,
    ) -> PolicyDecision {
        if !spec.invokers.agent {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!("`{}` is not open to agents", spec.name),
            };
        }
        if may_write == Some(false) {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!(
                    "`{}` changes state, and thread {} may not write (only the stream's writer thread can)",
                    spec.name,
                    thread_id.map(|t| t.to_string()).unwrap_or_default()
                ),
            };
        }
        PolicyDecision::Allow
    }

    /// May `thread_id` run `intent` now? Allows when the thread is
    /// unknown (nothing to enforce against).
    pub async fn check_tool(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        intent: &ToolIntent<'_>,
    ) -> PolicyDecision {
        if intent.kind != IntentKind::WorktreeWrite {
            return PolicyDecision::Allow;
        }
        let Some(thread) = svc.thread_store.get(thread_id).await.ok().flatten() else {
            return PolicyDecision::Allow;
        };
        let project_dir = svc.layout.project_dir.as_path();
        // The thread's own tree is its stream's worktree (a sibling dir for
        // a worktree stream); every other stream's tree is off limits.
        let streams = oxplow_domain::stores::StreamStore::list(svc.stream_store.as_ref())
            .await
            .unwrap_or_default();
        let worktree_root = svc.thread_worktree(thread_id).await;
        let other_roots: Vec<std::path::PathBuf> = streams
            .iter()
            .filter(|s| s.id != thread.stream_id && !s.worktree_path.is_empty())
            .map(|s| std::path::PathBuf::from(&s.worktree_path))
            .chain(std::iter::once(project_dir.to_path_buf()))
            .filter(|p| p != &worktree_root)
            .collect();
        decide_tool(
            intent,
            &PolicyFacts {
                thread: &thread,
                worktree_root: &worktree_root,
                other_roots: &other_roots,
                project_dir,
            },
        )
    }
}

/// The tool names that start a subagent. One list, shared with the status
/// derivation.
pub const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_intent_takes_the_first_string_path() {
        let i = claude_intent(&serde_json::json!({
            "tool_name": "NotebookEdit",
            "tool_input": {"file_path": null, "notebook_path": "nb.ipynb"}
        }))
        .unwrap();
        assert_eq!(i.paths, vec!["nb.ipynb".to_string()]);
    }

    #[test]
    fn claude_intent_gates_exactly_the_guarded_tools() {
        // Only the four structured edits can be refused; everything else
        // skips the policy's lookups (tsk: pre_tool_check fast path).
        for t in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
            let i = claude_intent(
                &serde_json::json!({"tool_name": t, "tool_input": {"file_path": "src/a.rs"}}),
            )
            .unwrap();
            assert_eq!(
                (i.label.as_str(), i.kind, i.paths.clone()),
                (t, IntentKind::WorktreeWrite, vec!["src/a.rs".to_string()])
            );
        }
        for t in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "Task",
            "WebFetch",
            "WebSearch",
            "TodoWrite",
            "mcp__oxplow__run_command",
            "",
        ] {
            assert!(
                claude_intent(&serde_json::json!({"tool_name": t})).is_none(),
                "{t} must short-circuit"
            );
        }
        // The gate admits exactly the write guard's tool set.
        use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
        for t in WORKTREE_MUTATING_TOOLS {
            assert!(
                claude_intent(&serde_json::json!({"tool_name": t})).is_some(),
                "gate must admit {t}"
            );
        }
        // notebook_path / path are read when file_path is absent.
        let nb = claude_intent(&serde_json::json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "n.ipynb"}})).unwrap();
        assert_eq!(nb.paths, vec!["n.ipynb".to_string()]);
    }
}
