//! Agent system-prompt assembly.
//!
//! Combines (in order):
//! 1. The repo-level `CLAUDE.md` (project instructions).
//! 2. An Oxplow session-context note describing the active stream/thread.
//! 3. The thread's `custom_prompt` if set.
//! 4. The stream's `custom_prompt` if set.
//! 5. The user's `agentPromptAppend` from `.oxplow/project.yaml`.
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

/// Build the visible Oxplow session-context note. Includes worktree
/// path, stream/thread titles, identifiers, and the practical effect
/// of the thread's writer/read-only role.
pub fn build_session_context_block(stream: &Stream, thread: Option<&Thread>) -> String {
    build_session_context_block_with_role(stream, thread, None)
}

/// Like `build_session_context_block` but appends a prominent role-
/// change note before `</session-context>` when the current
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
    let mut s = String::from(
        "<session-context>\n## Oxplow session context\n\n\
         Oxplow is providing the agent's current workspace assignment. This note is refreshed after a session restart or when the assignment changes.\n\n",
    );
    s.push_str(&format!(
        "- **Stream:** {} (`{}`)\n",
        stream.title, stream.id
    ));
    s.push_str(&format!("- **Worktree:** `{}`\n", stream.worktree_path));
    s.push_str(&format!("- **Branch:** `{}`\n", stream.branch));
    if let Some(t) = thread {
        s.push_str(&format!("- **Thread:** {} (`{}`)\n", t.title, t.id));
        let current = RoleMode::from_thread(t);
        s.push_str(&format!(
            "- **Access:** **{}** — {}\n",
            current.as_str(),
            role_description(current)
        ));
        if let Some(initial) = initial_role {
            if initial != current {
                s.push('\n');
                s.push_str(&role_change_banner(initial, current));
                s.push('\n');
            }
        }
    }
    s.push_str("</session-context>");
    s
}

fn role_description(role: RoleMode) -> &'static str {
    match role {
        RoleMode::Writer => "may edit project files",
        RoleMode::ReadOnly => "may inspect the project, but project file edits are blocked",
    }
}

/// Loud banner emitted when the thread's role flipped mid-session.
/// Phrased so the agent treats it as a direct override of the Access line
/// in the session context it started with.
pub fn role_change_banner(initial: RoleMode, current: RoleMode) -> String {
    match (initial, current) {
        (RoleMode::ReadOnly, RoleMode::Writer) => "**Access changed:** This thread was promoted to writer after the session started. It may now edit the worktree; the earlier read-only instruction no longer applies.".to_string(),
        (RoleMode::Writer, RoleMode::ReadOnly) => "**Access changed:** This thread was changed to read-only after the session started. Project file edits are now blocked; wiki pages are still written with `oxplow.knowledge.write_page`.".to_string(),
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
    assemble(project_dir, config, stream, thread, None)
}

/// The system prompt for an ACP agent: the same, minus the
/// `<session-context>` block. An ACP session's first human prompt always
/// carries a fresh one (`AgentContext::prompt_context`), so including it
/// here too would send it twice. It has no skill files to discover, so it
/// gets `skills`' index instead (`capabilities::agent_text`).
pub fn assemble_acp_system_prompt(
    project_dir: &Path,
    config: &OxplowConfig,
    stream: &Stream,
    thread: Option<&Thread>,
    skills: &oxplow_plugin::AgentText,
) -> String {
    assemble(project_dir, config, stream, thread, Some(skills))
}

/// `skills` is an ACP agent's index; a terminal agent (`None`) gets the
/// session context instead and discovers its skill files.
fn assemble(
    project_dir: &Path,
    config: &OxplowConfig,
    stream: &Stream,
    thread: Option<&Thread>,
    skills: Option<&oxplow_plugin::AgentText>,
) -> String {
    let session_context = skills.is_none();
    let mut out = String::new();
    let claude_md = load_claude_md(project_dir);
    if !claude_md.is_empty() {
        out.push_str(&claude_md);
        out.push_str("\n\n");
    }
    if session_context {
        out.push_str(&build_session_context_block(stream, thread));
        out.push_str("\n\n");
    }
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
        .testing
        .agent_hint
        .as_deref()
        .filter(|h| !h.is_empty())
    {
        out.push_str("\n# Testing\n");
        out.push_str(hint);
        out.push('\n');
    }
    if let Some(text) = skills {
        // An ACP agent: no skill files to discover (tsk376).
        out.push_str(&skill_index_block(text));
    }
    out.trim_end().to_string()
}

/// The oxplow skills for an agent that can't discover skill files: one
/// line each, to be fetched with the MCP `get_skill` tool when the work
/// matches.
fn skill_index_block(text: &oxplow_plugin::AgentText) -> String {
    let mut out = String::from(
        "\n# oxplow skills\nBefore doing work one of these describes, call the oxplow MCP tool \
         `get_skill` with its name and follow what it says.\n",
    );
    for (name, description) in text.skill_index() {
        out.push_str(&format!("- {name}: {description}\n"));
    }
    out
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
            metric_retention_days: 0,
            metric_detail_max_per_producer: 100,
            metric_detail_retention_days: 30,
            generated: oxplow_config::GeneratedConfig::default(),
            snapshot_max_file_bytes: 1_000_000,
            snapshot_turn_budget_ms: 2000,
            symbols_max_files_per_snapshot: oxplow_config::DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT,
            inject_session_context: true,
            icon_tint: None,
            testing: Default::default(),
            metrics: Default::default(),
            collectors: Default::default(),
            collectors_yaml: None,
            measures: Default::default(),
            dimensions: Default::default(),
            zones: Default::default(),
            agent_models: Default::default(),
            acp_agents: Vec::new(),
            extension_instances: Default::default(),
            active_providers: Default::default(),
            personal_active_providers: Default::default(),
            replacements_off: Default::default(),
            event_retention: Default::default(),
            ai_roles: Default::default(),
            extensions_disabled: Vec::new(),
        }
    }

    fn stream() -> Stream {
        Stream {
            id: StreamId::new(1),
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
            id: ThreadId::new(1),
            stream_id: StreamId::new(1),
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
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
        assert!(block.contains("## Oxplow session context"));
        assert!(block.contains("**Stream:** oxplow (`str1`)"));
        assert!(block.contains("**Worktree:** `/repo`"));
        assert!(block.contains("**Branch:** `main`"));
        assert!(block.contains("**Thread:** explore (`thr1`)"));
        assert!(block.contains("**Access:** **writer**"));
        assert!(block.contains("after a session restart or when the assignment changes"));
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
    fn testing_agent_hint_appended_when_set() {
        let dir = tempdir().unwrap();
        let mut cfg = config();
        cfg.testing.agent_hint = Some("Run tests with bun run test:collect".into());
        let prompt = assemble_system_prompt(dir.path(), &cfg, &stream(), Some(&thread()));
        assert!(prompt.contains("# Testing\n"));
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
        assert!(block.contains("**Access:** **read-only**"));
        assert!(block.contains("project file edits are blocked"));
    }

    #[test]
    fn role_change_banner_fires_on_promotion() {
        // Thread is currently writer; was launched as read-only.
        let block = build_session_context_block_with_role(
            &stream(),
            Some(&thread()),
            Some(RoleMode::ReadOnly),
        );
        assert!(block.contains("**Access:** **writer**"));
        assert!(block.contains("**Access changed:**"));
        assert!(block.contains("promoted to writer"));
        assert!(block.contains("earlier read-only instruction no longer applies"));
    }

    #[test]
    fn role_change_banner_fires_on_demotion() {
        // Thread is currently read-only; was launched as writer.
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let block =
            build_session_context_block_with_role(&stream(), Some(&t), Some(RoleMode::Writer));
        assert!(block.contains("**Access:** **read-only**"));
        assert!(block.contains("**Access changed:**"));
        assert!(block.contains("changed to read-only"));
    }

    #[test]
    fn no_banner_when_role_matches_initial() {
        // Initial=Writer, current=Writer → steady state, no banner.
        let block = build_session_context_block_with_role(
            &stream(),
            Some(&thread()),
            Some(RoleMode::Writer),
        );
        assert!(block.contains("**Access:** **writer**"));
        assert!(!block.contains("**Access changed:**"));
    }

    #[test]
    fn no_banner_when_initial_role_unset() {
        // Caller hasn't captured an initial role yet (e.g. very first
        // turn or hook fired before capture) — no banner.
        let block = build_session_context_block_with_role(&stream(), Some(&thread()), None);
        assert!(!block.contains("**Access changed:**"));
    }

    #[test]
    fn the_acp_system_prompt_leaves_session_context_to_the_first_prompt() {
        let dir = tempdir().unwrap();
        let full = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        let acp = assemble_acp_system_prompt(
            dir.path(),
            &config(),
            &stream(),
            Some(&thread()),
            &oxplow_plugin::AgentText::core(),
        );
        assert!(full.contains("<session-context>"));
        assert!(!acp.contains("<session-context>"), "{acp}");
        assert!(acp.contains("be precise"));
    }

    /// An ACP agent can't discover skill files, so its prompt indexes
    /// them for `get_skill`; a terminal agent's runtime ships the files
    /// (tsk376).
    #[test]
    fn the_acp_system_prompt_indexes_the_skills() {
        let dir = tempdir().unwrap();
        let full = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        let acp = assemble_acp_system_prompt(
            dir.path(),
            &config(),
            &stream(),
            Some(&thread()),
            &oxplow_plugin::AgentText::core(),
        );
        assert!(acp.contains("get_skill"), "{acp}");
        assert!(
            acp.contains("- oxplow-extension: Build oxplow lenses"),
            "{acp}"
        );
        assert!(!full.contains("get_skill"));
    }
}
