//! The policy every agent transport enforces: the write guard and filing
//! enforcement, with the I/O they need (the thread, the stream's claim,
//! git state). Decisions are transport-neutral: the hook route renders
//! them as Claude's JSON, and the ACP client as permission answers. The
//! pure rules live in `oxplow_runtime::policy`. A turn's end is never
//! refused ([work-tracking.md](../../../.context/work-tracking.md)). See
//! `.context/agent-model.md`.

use std::path::Path;

use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{Thread, ThreadId};
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
    use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
    use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    if !(WORKTREE_MUTATING_TOOLS.contains(&tool_name)
        || ALWAYS_WRITE_INTENT_TOOL_NAMES.contains(&tool_name))
    {
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
        // Only the writer is subject to filing; skip its lookups otherwise.
        let (has_open_effort, git_operation_in_progress) = if thread.status.is_writer() {
            (
                stream_has_open_effort(svc, &thread).await,
                git_operation_in_progress(&worktree_root),
            )
        } else {
            (false, false)
        };
        let active_work_items = svc.work_items.active();
        decide_tool(
            intent,
            &PolicyFacts {
                thread: &thread,
                worktree_root: &worktree_root,
                other_roots: &other_roots,
                project_dir,
                has_open_effort,
                git_operation_in_progress,
                active_work_items: &active_work_items,
            },
        )
    }
}

/// The tool names that start a subagent. One list, shared with the status
/// derivation.
pub const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

/// Whether the writer's stream has an open effort — the filing guard's
/// claim (P2.7, tsk431). An effort opens when a task is filed or moved
/// `in_progress` (in the same transaction) or through `effort.open` for
/// another provider's work item; an `in_progress` row alone isn't one.
///
/// Scoped to the whole STREAM, not just the literal thread (tsk133): a
/// stream has exactly one active writer (the `idx_threads_one_active_per_stream`
/// unique index + the write guard), so an effort on *any* of its threads
/// is a legitimate claim for the writer — cross-thread dispatch needs no
/// `move_task` first. Queued/closed threads still can't write (the write
/// guard runs first). A lookup failure denies (fails closed).
pub(crate) async fn stream_has_open_effort(svc: &Services, thread: &Thread) -> bool {
    svc.effort_store
        .stream_has_open_effort(thread.stream_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "filing: open-effort lookup failed");
            false
        })
}

/// Returns true when the worktree is mid git merge / rebase /
/// cherry-pick / revert. Filing enforcement exempts edits in these
/// states because conflict resolution can't dead-lock against the
/// filing rule. Mirrors `src/electron/filing-enforcement.ts`.
pub fn git_operation_in_progress(project_dir: &Path) -> bool {
    let gitdir = project_dir.join(".git");
    for marker in [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        if gitdir.join(marker).exists() {
            return true;
        }
    }
    // Worktrees: .git is a file pointing at the real gitdir.
    if let Ok(contents) = std::fs::read_to_string(&gitdir) {
        if let Some(real_dir) = contents.strip_prefix("gitdir: ") {
            let real = Path::new(real_dir.trim());
            for marker in [
                "MERGE_HEAD",
                "REBASE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
            ] {
                if real.join(marker).exists() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;
    use oxplow_domain::stores::TaskStore as _;

    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{
        AgentKind, Stream, StreamId, StreamKind, Task, TaskActorKind, TaskId, TaskPriority,
        TaskStatus, ThreadStatus, Timestamp,
    };

    /// In-memory services over a real git repo (the session layer refuses
    /// non-git dirs).
    fn services() -> (Services, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        (Services::in_memory(dir.path()).unwrap(), dir)
    }

    fn test_stream() -> Stream {
        Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Worktree,
            title: "feat".into(),
            branch: "feat".into(),
            branch_ref: "refs/heads/feat".into(),
            branch_source: "main".into(),
            worktree_path: "/repo/wt".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    fn test_thread(stream_id: StreamId, status: ThreadStatus) -> Thread {
        Thread {
            id: ThreadId::placeholder(),
            stream_id,
            title: "thread".into(),
            status,
            sort_index: 0,
            pane_target: "working".into(),
            agent: AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            archived_at: None,
        }
    }

    fn in_progress_task(thread_id: ThreadId) -> Task {
        Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread_id),
            parent_id: None,
            title: "work".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        }
    }

    /// The primary stream and its seeded active (writer) thread, which
    /// `Services::in_memory` creates via `ensure_primary` at boot.
    async fn primary_writer(svc: &Services) -> (Stream, Thread) {
        let stream = svc.stream_store.primary().await.unwrap().unwrap();
        let writer = svc
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.status == ThreadStatus::Active)
            .expect("primary stream has a seeded active writer thread");
        (stream, writer)
    }

    /// A non-string `file_path` doesn't hide the path under another key
    /// (tsk371).
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
        // The gate admits exactly the union of the two guards' tool sets.
        use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
        use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
        for t in WORKTREE_MUTATING_TOOLS
            .iter()
            .chain(ALWAYS_WRITE_INTENT_TOOL_NAMES.iter())
        {
            assert!(
                claude_intent(&serde_json::json!({"tool_name": t})).is_some(),
                "gate must admit {t}"
            );
        }
        // notebook_path / path are read when file_path is absent.
        let nb = claude_intent(&serde_json::json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "n.ipynb"}})).unwrap();
        assert_eq!(nb.paths, vec!["n.ipynb".to_string()]);
    }

    #[test]
    fn git_op_in_progress_detects_merge_head() {
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
        fs::write(tmp.path().join(".git/MERGE_HEAD"), b"deadbeef\n").unwrap();
        assert!(git_operation_in_progress(tmp.path()));
    }

    #[test]
    fn git_op_in_progress_detects_each_marker() {
        use std::fs;
        for marker in ["REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            let tmp = tempfile::TempDir::new().unwrap();
            fs::create_dir_all(tmp.path().join(".git")).unwrap();
            assert!(!git_operation_in_progress(tmp.path()));
            fs::write(tmp.path().join(".git").join(marker), b"deadbeef\n").unwrap();
            assert!(
                git_operation_in_progress(tmp.path()),
                "expected {marker} to count"
            );
        }
    }

    #[test]
    fn git_op_in_progress_follows_worktree_gitdir_pointer() {
        // In a secondary worktree, `.git` is a *file* pointing at the
        // real gitdir. The function must follow that pointer so a
        // mid-merge worktree still trips the carve-out.
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        let real_gitdir = tmp.path().join("real-gitdir");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(&real_gitdir).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real_gitdir.display()),
        )
        .unwrap();
        assert!(!git_operation_in_progress(&worktree));
        fs::write(real_gitdir.join("MERGE_HEAD"), b"x\n").unwrap();
        assert!(git_operation_in_progress(&worktree));
    }

    #[test]
    fn git_op_in_progress_no_dot_git_returns_false() {
        // Bare directory with no .git at all — function must not
        // panic and must report no-op.
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
    }

    /// P2.7 (tsk431): the claim is an OPEN EFFORT in the stream, on any of
    /// its threads (tsk133 — cross-thread dispatch keeps working).
    #[tokio::test]
    async fn an_effort_on_a_sibling_thread_satisfies_the_writer() {
        let (svc, _dir) = services();
        let (stream, writer_b) = primary_writer(&svc).await;
        let sibling_a_id = svc
            .thread_store
            .upsert(&test_thread(stream.id, ThreadStatus::Queued))
            .await
            .unwrap();
        let task = svc
            .task_store
            .insert(&in_progress_task(sibling_a_id))
            .await
            .unwrap();
        svc.effort_store
            .start(&work_item_ref(task), &sibling_a_id, None)
            .await
            .unwrap();
        assert!(stream_has_open_effort(&svc, &writer_b).await);
    }

    /// Another provider's work item is a claim too (`effort.open`).
    #[tokio::test]
    async fn an_effort_on_a_foreign_work_item_satisfies_the_writer() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        svc.effort_store
            .start("work_item:issues:ENG-12", &writer.id, None)
            .await
            .unwrap();
        assert!(stream_has_open_effort(&svc, &writer).await);
    }

    /// An `in_progress` row alone is not a claim — only its effort is.
    #[tokio::test]
    async fn an_in_progress_task_without_an_effort_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        svc.task_store
            .insert(&in_progress_task(writer.id))
            .await
            .unwrap();
        assert!(!stream_has_open_effort(&svc, &writer).await);
    }

    #[tokio::test]
    async fn an_effort_in_another_stream_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        let other_stream_id = svc.stream_store.upsert(&test_stream()).await.unwrap();
        let other_thread_id = svc
            .thread_store
            .upsert(&test_thread(other_stream_id, ThreadStatus::Active))
            .await
            .unwrap();
        svc.effort_store
            .start("work_item:issues:ENG-9", &other_thread_id, None)
            .await
            .unwrap();
        assert!(!stream_has_open_effort(&svc, &writer).await);
    }

    #[tokio::test]
    async fn no_claim_anywhere_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        assert!(
            !stream_has_open_effort(&svc, &writer).await,
            "no open effort anywhere → guard not satisfied"
        );
    }
}
