//! Agent system-prompt assembly.
//!
//! Combines (in order):
//! 1. What every agent must know of oxplow
//!    (`oxplow_agent_text::WORKING_IN_OXPLOW`).
//! 2. The `<session-context>` note, for a harness whose prompt hooks can't
//!    carry it (otherwise the first prompt's context does).
//! 3. The thread's `custom_prompt` if set.
//! 4. The stream's `custom_prompt` if set.
//! 5. The user's `agentPromptAppend` from `.oxplow/project.yaml`.
//! 6. The project's testing hint.
//! 7. The skill index, for a harness that ships no skill files.
//!
//! A project's own instruction files (`AGENTS.md`, `CLAUDE.md`) are the
//! harness's to read: oxplow never reads or forwards them.
//!
//! The combined string is passed through the selected agent's
//! system-prompt mechanism.

use oxplow_config::OxplowConfig;
use oxplow_domain::{Stream, Thread, ThreadId};

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

/// Build the visible Oxplow session-context note. Includes worktree
/// path, stream/thread titles, identifiers, and the practical effect
/// of the thread's writer/read-only role.
pub fn build_session_context_block(stream: &Stream, thread: Option<&Thread>) -> String {
    build_session_context_block_with_role(stream, thread, None)
}

/// Like `build_session_context_block` but appends a prominent role-
/// change note before `</session-context>` when the current
/// thread role differs from the supplied `initial_role`: the role the
/// agent was first told is in its context for the rest of the session,
/// so a promotion or demotion has to say it overrides that. No banner is
/// emitted when the role hasn't changed (steady-state turns don't grow).
pub fn build_session_context_block_with_role(
    stream: &Stream,
    thread: Option<&Thread>,
    initial_role: Option<RoleMode>,
) -> String {
    build_session_context_block_full(stream, thread, initial_role, None)
}

/// The agent running on the stream's writer thread, so a read-only
/// thread can match it against what `ListAgents` shows. Agent-neutral:
/// the harness key, the OS pid and the harness's own session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterAgent {
    pub thread_title: String,
    pub thread_id: ThreadId,
    pub harness: String,
    pub pid: Option<u32>,
    /// The harness's own session id; empty when unknown.
    pub session: String,
}

/// [`build_session_context_block_with_role`] plus, on a read-only
/// thread, a line naming the live agent on the stream's writer thread.
pub fn build_session_context_block_full(
    stream: &Stream,
    thread: Option<&Thread>,
    initial_role: Option<RoleMode>,
    writer: Option<&WriterAgent>,
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
        if let (RoleMode::ReadOnly, Some(w)) = (current, writer) {
            s.push_str(&format!(
                "- **Writer thread:** {} (`{}`) — agent `{}`",
                w.thread_title, w.thread_id, w.harness
            ));
            if let Some(pid) = w.pid {
                s.push_str(&format!(", pid {pid}"));
            }
            if !w.session.is_empty() {
                s.push_str(&format!(", session `{}`", w.session));
            }
            s.push('\n');
        }
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

/// Compose the system prompt oxplow adds to whatever the agent's own is,
/// for a session of `harness` (its declared implementation). Sections are
/// separated by blank lines.
///
/// What the harness delivers decides two parts: the `<session-context>`
/// block is here only when no prompt hook can carry it (`prompt_context`;
/// otherwise the first prompt's context does, and both would send it
/// twice), and the skills are indexed here for `get_skill` only when the
/// harness ships no skill files (`runtime_text`).
pub fn assemble_system_prompt(
    config: &OxplowConfig,
    stream: &Stream,
    thread: Option<&Thread>,
    harness: &crate::capabilities::Implementation,
    skills: &oxplow_domain::agent::text::AgentText,
) -> String {
    let mut out = String::new();
    out.push_str(oxplow_agent_text::WORKING_IN_OXPLOW.trim());
    out.push_str("\n\n");
    if !harness.has_feature(crate::agent_context::PROMPT_CONTEXT) {
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
    if !harness.has_feature(RUNTIME_TEXT) {
        out.push_str(&skill_index_block(skills));
    }
    out.trim_end().to_string()
}

/// The agent-harness feature: it ships skill files and slash commands its
/// agent discovers.
const RUNTIME_TEXT: &str = "runtime_text";

/// The oxplow skills for an agent that can't discover skill files: one
/// line each, to be fetched with the MCP `get_skill` tool when the work
/// matches.
fn skill_index_block(text: &oxplow_domain::agent::text::AgentText) -> String {
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
    use oxplow_domain::{StreamId, StreamKind, ThreadId, ThreadStatus, Timestamp};
    use tempfile::tempdir;

    /// A harness declaring `features`.
    fn harness(features: &[&str]) -> crate::capabilities::Implementation {
        crate::capabilities::Implementation {
            capability: "agent_harness".into(),
            id: "h".into(),
            title: "H".into(),
            extension: None,
            source: crate::capabilities::Source::Core,
            features: serde_json::Value::Object(
                features
                    .iter()
                    .map(|f| (f.to_string(), serde_json::Value::Bool(true)))
                    .collect(),
            ),
            fields: serde_json::json!([]),
            id_pattern: None,
            config: serde_json::json!({}),
        }
    }

    /// A terminal harness whose hooks carry context and that ships skill
    /// files (Claude Code's, Codex's).
    fn full() -> crate::capabilities::Implementation {
        harness(&["runtime_text", "prompt_context", "tool_context"])
    }

    fn prompt_for(
        config: &OxplowConfig,
        stream: &Stream,
        h: &crate::capabilities::Implementation,
    ) -> String {
        assemble_system_prompt(
            config,
            stream,
            Some(&thread()),
            h,
            &oxplow_agent_text::core_text(),
        )
    }

    fn config() -> OxplowConfig {
        OxplowConfig {
            agents: vec![],
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
            inject_session_context: true,
            icon_tint: None,
            testing: Default::default(),
            scripts: Default::default(),
            metrics: Default::default(),
            zones: Default::default(),
            agent_config: Default::default(),
            acp_agents: Vec::new(),
            extension_instances: Default::default(),
            active_providers: Default::default(),
            personal_active_providers: Default::default(),
            personal_new_thread_session: None,
            replacements: Default::default(),
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
            host: oxplow_domain::HostId::LOCAL,
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
        let prompt = prompt_for(&config(), &stream(), &harness(&["runtime_text"]));
        assert!(prompt.contains("<session-context>"));
        assert!(prompt.contains("Use TDD"));
        assert!(prompt.contains("be precise"));
    }

    #[test]
    fn testing_agent_hint_appended_when_set() {
        let mut cfg = config();
        cfg.testing.agent_hint = Some("Run tests with bun run test:collect".into());
        let prompt = prompt_for(&cfg, &stream(), &full());
        assert!(prompt.contains("# Testing\n"));
        assert!(prompt.contains("bun run test:collect"));
    }

    #[test]
    fn collection_agent_hint_absent_when_unset() {
        let prompt = prompt_for(&config(), &stream(), &full());
        assert!(!prompt.contains("# Collection"));
    }

    /// What every agent must know of oxplow rides in the prompt oxplow
    /// adds, terminal and ACP alike: no project has to repeat it in its
    /// own instruction files.
    #[test]
    fn every_prompt_carries_how_to_work_in_oxplow() {
        for prompt in [
            prompt_for(&config(), &stream(), &full()),
            prompt_for(&config(), &stream(), &harness(&["prompt_context"])),
            prompt_for(&config(), &stream(), &harness(&[])),
        ] {
            assert!(
                prompt.contains(oxplow_agent_text::WORKING_IN_OXPLOW.trim()),
                "{prompt}"
            );
        }
        assert!(oxplow_agent_text::WORKING_IN_OXPLOW.contains("end it with the question"));
        assert!(oxplow_agent_text::WORKING_IN_OXPLOW.contains("oxplow-runtime"));
    }

    /// A project's instruction files are its harness's to read: the
    /// prompt oxplow adds carries none of them, whatever the project has.
    #[test]
    fn the_prompt_carries_no_project_instruction_files() {
        let dir = tempdir().unwrap();
        for file in ["AGENTS.md", "CLAUDE.md"] {
            std::fs::write(dir.path().join(file), "## Repo rules\nRule 1.").unwrap();
        }
        let mut s = stream();
        s.worktree_path = dir.path().to_string_lossy().into_owned();
        let prompt = prompt_for(&config(), &s, &full());
        assert!(!prompt.contains("Repo rules"), "{prompt}");
    }

    #[test]
    fn read_only_thread_marks_role() {
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let block = build_session_context_block(&stream(), Some(&t));
        assert!(block.contains("**Access:** **read-only**"));
        assert!(block.contains("project file edits are blocked"));
    }

    fn writer_agent() -> WriterAgent {
        WriterAgent {
            thread_title: "Thread".into(),
            thread_id: ThreadId::new(1),
            harness: "claude".into(),
            pid: Some(11684),
            session: "abc-123".into(),
        }
    }

    #[test]
    fn a_read_only_thread_is_told_who_the_writer_agent_is() {
        let mut t = thread();
        t.id = ThreadId::new(2);
        t.status = ThreadStatus::Queued;
        let block =
            build_session_context_block_full(&stream(), Some(&t), None, Some(&writer_agent()));
        assert!(
            block.contains(
                "**Writer thread:** Thread (`thr1`) — agent `claude`, pid 11684, session `abc-123`"
            ),
            "{block}"
        );
    }

    #[test]
    fn a_writer_thread_is_not_told_of_a_writer() {
        let block = build_session_context_block_full(
            &stream(),
            Some(&thread()),
            None,
            Some(&writer_agent()),
        );
        assert!(!block.contains("Writer thread"));
    }

    #[test]
    fn unknown_writer_agent_parts_are_left_out() {
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let mut w = writer_agent();
        w.pid = None;
        w.session = String::new();
        let block = build_session_context_block_full(&stream(), Some(&t), None, Some(&w));
        assert!(
            block.contains("**Writer thread:** Thread (`thr1`) — agent `claude`\n"),
            "{block}"
        );
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

    /// The session context is sent once: by the first prompt's hook when
    /// the harness's prompt hooks carry context, else in the launch prompt
    /// (whose text is frozen, so a harness without the channel never sees
    /// it change).
    #[test]
    fn the_session_context_rides_the_launch_prompt_only_without_a_prompt_channel() {
        let with = prompt_for(&config(), &stream(), &full());
        assert!(!with.contains("<session-context>"), "{with}");
        assert!(with.contains("be precise"));
        let without = prompt_for(&config(), &stream(), &harness(&["runtime_text"]));
        assert!(without.contains("<session-context>"), "{without}");
    }

    /// A harness that ships no skill files (an ACP agent) gets the skills
    /// indexed in its prompt, fetched with `get_skill`; one that ships them
    /// doesn't.
    #[test]
    fn the_prompt_indexes_the_skills_only_without_skill_files() {
        let indexed = prompt_for(&config(), &stream(), &harness(&["prompt_context"]));
        assert!(indexed.contains("get_skill"), "{indexed}");
        assert!(
            indexed.contains("- oxplow-extension: Build oxplow lenses"),
            "{indexed}"
        );
        assert!(!prompt_for(&config(), &stream(), &full()).contains("get_skill"));
    }
}
