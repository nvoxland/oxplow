//! Agent system-prompt assembly.
//!
//! Combines (in order):
//! 1. The repo-level `CLAUDE.md` (project instructions).
//! 2. A `<session-context>` block describing the active stream/thread.
//! 3. The thread's `custom_prompt` if set.
//! 4. The stream's `custom_prompt` if set.
//! 5. The user's `agentPromptAppend` from `oxplow.yaml`.
//!
//! The combined string is passed through the selected agent's
//! system-prompt mechanism.

use std::path::Path;

use oxplow_config::OxplowConfig;
use oxplow_domain::{Stream, Thread};

/// The two role buckets the agent cares about. Writer threads can
/// mutate the worktree; read-only threads cannot (their PreToolUse
/// hook denies Edit/Write/MultiEdit/NotebookEdit). Used as the
/// comparison baseline for the ROLE CHANGE banner — captured once
/// per Claude session id at agent launch, then compared against the
/// current thread status on every UserPromptSubmit / qualifying
/// PostToolUse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleMode {
    Writer,
    ReadOnly,
}

impl RoleMode {
    pub fn from_thread(thread: &Thread) -> Self {
        if thread.status.is_writer() {
            RoleMode::Writer
        } else {
            RoleMode::ReadOnly
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RoleMode::Writer => "writer",
            RoleMode::ReadOnly => "read-only",
        }
    }
}

/// Read `<project>/CLAUDE.md` if it exists. Empty string otherwise.
pub fn load_claude_md(project_dir: &Path) -> String {
    let path = project_dir.join("CLAUDE.md");
    std::fs::read_to_string(&path).unwrap_or_default()
}

/// Build the `<session-context>` block. Includes worktree path,
/// stream/thread titles, and the thread's writer/read-only role.
pub fn build_session_context_block(stream: &Stream, thread: Option<&Thread>) -> String {
    build_session_context_block_with_role(stream, thread, None)
}

/// Like `build_session_context_block` but appends a loud
/// `ROLE CHANGE:` line before `</session-context>` when the current
/// thread role differs from the supplied `initial_role`. The launch-
/// time `NON_WRITER_PROMPT_BLOCK` is frozen in the system prompt and
/// replayed via cache-read on every turn — without this banner, a
/// mid-session writer promotion never reaches the agent. The banner
/// supersedes the stale block in-place. No banner is emitted when
/// the role hasn't changed (steady-state turns don't grow).
pub fn build_session_context_block_with_role(
    stream: &Stream,
    thread: Option<&Thread>,
    initial_role: Option<RoleMode>,
) -> String {
    let mut s = String::from("<session-context>\n");
    s.push_str(&format!("stream_id={}\n", stream.id));
    s.push_str(&format!("stream_title={}\n", stream.title));
    s.push_str(&format!("worktree_path={}\n", stream.worktree_path));
    s.push_str(&format!("branch={}\n", stream.branch));
    if let Some(t) = thread {
        s.push_str(&format!("thread_id={}\n", t.id));
        s.push_str(&format!("thread_title={}\n", t.title));
        let current = RoleMode::from_thread(t);
        s.push_str(&format!("role={}\n", current.as_str()));
        if let Some(initial) = initial_role {
            if initial != current {
                s.push_str(&role_change_banner(initial, current));
                s.push('\n');
            }
        }
    }
    s.push_str("</session-context>");
    s
}

/// Loud banner emitted when the thread's role flipped mid-session.
/// Phrased so the agent treats it as a direct override of the
/// frozen `NON_WRITER_PROMPT_BLOCK` (or absence thereof) in the
/// initial system prompt.
pub fn role_change_banner(initial: RoleMode, current: RoleMode) -> String {
    match (initial, current) {
        (RoleMode::ReadOnly, RoleMode::Writer) => "ROLE CHANGE: this thread has been promoted to writer mid-session. The NON_WRITER block in your initial system prompt is SUPERSEDED — you may now use Write/Edit/Bash to mutate the worktree. File a tracked task before editing project files (filing-enforcement applies).".to_string(),
        (RoleMode::Writer, RoleMode::ReadOnly) => "ROLE CHANGE: this thread has been demoted to read-only mid-session. The NON_WRITER block applies now even though it wasn't in your initial system prompt — Write/Edit/Bash mutations to the worktree will be blocked. Wiki captures under .oxplow/wiki/ remain allowed.".to_string(),
        // Same-role pairs never reach this fn — caller skips.
        _ => String::new(),
    }
}

/// Compose the full system-prompt suffix oxplow appends to whatever
/// the agent's built-in system prompt is. Sections are separated by
/// blank lines so Claude renders each as its own block.
pub fn assemble_system_prompt(
    project_dir: &Path,
    config: &OxplowConfig,
    stream: &Stream,
    thread: Option<&Thread>,
) -> String {
    let mut out = String::new();
    let claude_md = load_claude_md(project_dir);
    if !claude_md.is_empty() {
        out.push_str(&claude_md);
        out.push_str("\n\n");
    }
    out.push_str(&build_session_context_block(stream, thread));
    out.push_str("\n\n");
    if let Some(t) = thread {
        if let Some(prompt) = t.custom_prompt.as_deref().filter(|p| !p.is_empty()) {
            out.push_str(prompt);
            out.push_str("\n\n");
        }
    }
    if let Some(prompt) = stream.custom_prompt.as_deref().filter(|p| !p.is_empty()) {
        out.push_str(prompt);
        out.push_str("\n\n");
    }
    if !config.agent_prompt_append.is_empty() {
        out.push_str(&config.agent_prompt_append);
        out.push('\n');
    }
    if let Some(hint) = config
        .collection
        .agent_hint
        .as_deref()
        .filter(|h| !h.is_empty())
    {
        out.push_str("\n# Collection\n");
        out.push_str(hint);
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;
    use oxplow_domain::{StreamId, StreamKind, ThreadId, ThreadStatus, Timestamp};
    use tempfile::tempdir;

    fn config() -> OxplowConfig {
        OxplowConfig {
            agents: vec![AgentKind::Claude],
            project_name: "p".into(),
            lsp_servers: vec![],
            agent_prompt_append: "be precise".into(),
            snapshot_retention_days: 7,
            generated: vec![],
            snapshot_max_file_bytes: 1_000_000,
            inject_session_context: true,
            collection: Default::default(),
        }
    }

    fn stream() -> Stream {
        Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
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

    fn thread() -> Thread {
        Thread {
            id: ThreadId::from("b-1"),
            stream_id: StreamId::from("s-1"),
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: Some("Use TDD".into()),
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    #[test]
    fn session_context_includes_stream_and_thread_metadata() {
        let block = build_session_context_block(&stream(), Some(&thread()));
        assert!(block.contains("stream_id=s-1"));
        assert!(block.contains("thread_id=b-1"));
        assert!(block.contains("role=writer"));
    }

    #[test]
    fn assembled_prompt_concatenates_sections() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "## Repo rules\nRule 1.").unwrap();
        let prompt = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        assert!(prompt.contains("Repo rules"));
        assert!(prompt.contains("<session-context>"));
        assert!(prompt.contains("Use TDD"));
        assert!(prompt.contains("be precise"));
    }

    #[test]
    fn collection_agent_hint_appended_when_set() {
        let dir = tempdir().unwrap();
        let mut cfg = config();
        cfg.collection.agent_hint = Some("Run tests with bun run test:collect".into());
        let prompt = assemble_system_prompt(dir.path(), &cfg, &stream(), Some(&thread()));
        assert!(prompt.contains("# Collection\n"));
        assert!(prompt.contains("bun run test:collect"));
    }

    #[test]
    fn collection_agent_hint_absent_when_unset() {
        let dir = tempdir().unwrap();
        let prompt = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        assert!(!prompt.contains("# Collection"));
    }

    #[test]
    fn missing_claude_md_is_silent() {
        let dir = tempdir().unwrap();
        let prompt = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        // No "Repo rules" section but session-context still present.
        assert!(prompt.contains("<session-context>"));
    }

    #[test]
    fn read_only_thread_marks_role() {
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let block = build_session_context_block(&stream(), Some(&t));
        assert!(block.contains("role=read-only"));
    }

    #[test]
    fn role_change_banner_fires_on_promotion() {
        // Thread is currently writer; was launched as read-only.
        let block = build_session_context_block_with_role(
            &stream(),
            Some(&thread()),
            Some(RoleMode::ReadOnly),
        );
        assert!(block.contains("role=writer"));
        assert!(block.contains("ROLE CHANGE"));
        assert!(block.contains("promoted to writer"));
        assert!(block.contains("SUPERSEDED"));
    }

    #[test]
    fn role_change_banner_fires_on_demotion() {
        // Thread is currently read-only; was launched as writer.
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let block =
            build_session_context_block_with_role(&stream(), Some(&t), Some(RoleMode::Writer));
        assert!(block.contains("role=read-only"));
        assert!(block.contains("ROLE CHANGE"));
        assert!(block.contains("demoted to read-only"));
    }

    #[test]
    fn no_banner_when_role_matches_initial() {
        // Initial=Writer, current=Writer → steady state, no banner.
        let block = build_session_context_block_with_role(
            &stream(),
            Some(&thread()),
            Some(RoleMode::Writer),
        );
        assert!(block.contains("role=writer"));
        assert!(!block.contains("ROLE CHANGE"));
    }

    #[test]
    fn no_banner_when_initial_role_unset() {
        // Caller hasn't captured an initial role yet (e.g. very first
        // turn or hook fired before capture) — no banner.
        let block = build_session_context_block_with_role(&stream(), Some(&thread()), None);
        assert!(!block.contains("ROLE CHANGE"));
    }
}
