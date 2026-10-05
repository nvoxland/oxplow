//! Cross-surface parity manifest for oxplow's two adapter layers.
//!
//! Oxplow exposes domain operations on two independent adapters, both thin
//! wrappers over `oxplow_app::Services`:
//!   - **Tauri IPC** (`oxplow-tauri-ipc`) — `#[tauri::command]` fns the React
//!     UI calls.
//!   - **MCP** (`oxplow-mcp`) — rmcp `#[tool]`s the agent calls.
//!
//! These drifted silently: many user-meaningful ops lived on IPC but not MCP.
//! This manifest is the single source of truth for *which* surface each
//! capability should live on, and `tests/parity.rs` enforces that the real
//! registrations match it. Adding a `#[tauri::command]` or `#[tool]` without a
//! matching row fails the test, forcing an explicit classification.
//!
//! ## The four exposures
//! - [`Exposure::Both`] — present on IPC and MCP (names may diverge per surface).
//! - [`Exposure::UiOnly`] — intentionally UI-only, saying why an agent has
//!   no tool for it: a person's consent or setting, the person's own
//!   selection or input, live state pushed to the UI, runtime infra (menus,
//!   terminals, the LSP client, windows), an ancestry walk the VCS answers,
//!   or workspace file I/O the agent does with its own Read/Write tools.
//! - [`Exposure::Model`] — a UI read whose agent counterpart is published
//!   models: an agent reads the same rows with `query_sql`. Each model
//!   named must be published (`tests/parity.rs`).
//! - [`Exposure::AgentOnly`] — intentionally agent-only (dispatch, await_user,
//!   batch/orchestration affordances).
//!
//! Every row is decided: there is no "build the MCP tool later" exposure.
//! An agent's way to an IPC read is a tool (`Both`), a model (`Model`), or
//! none, with the reason (`UiOnly`).

/// Which adapter surface(s) a capability is expected to live on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Live on both the IPC (UI) and MCP (agent) surfaces.
    Both,
    /// Intentionally UI-only: `why` an agent has no tool for it.
    UiOnly { why: &'static str },
    /// A UI read an agent makes through these published models instead
    /// (`query_sql`).
    Model { models: &'static [&'static str] },
    /// Intentionally agent-only.
    AgentOnly,
}

/// One domain capability and the name it carries on each surface.
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    /// Stable human label — the parity row key. Must be unique.
    pub capability: &'static str,
    pub exposure: Exposure,
    /// IPC command name, or `None` when the capability is agent-only.
    pub ipc: Option<&'static str>,
    /// MCP tool name, or `None` when UI-only or read through models.
    pub mcp: Option<&'static str>,
}

use Exposure::*;

/// Helper for `Both` rows whose name is identical on both surfaces.
const fn both(name: &'static str) -> Capability {
    Capability {
        capability: name,
        exposure: Both,
        ipc: Some(name),
        mcp: Some(name),
    }
}
/// Helper for `Both` rows whose name diverges across surfaces.
const fn both_named(capability: &'static str, ipc: &'static str, mcp: &'static str) -> Capability {
    Capability {
        capability,
        exposure: Both,
        ipc: Some(ipc),
        mcp: Some(mcp),
    }
}
/// Helper for an intentionally UI-only command, and why.
const fn ui(name: &'static str, why: &'static str) -> Capability {
    Capability {
        capability: name,
        exposure: UiOnly { why },
        ipc: Some(name),
        mcp: None,
    }
}
/// Helper for a UI read whose agent counterpart is published models.
const fn model(name: &'static str, models: &'static [&'static str]) -> Capability {
    Capability {
        capability: name,
        exposure: Model { models },
        ipc: Some(name),
        mcp: None,
    }
}
/// Helper for an intentionally agent-only tool.
const fn agent(name: &'static str) -> Capability {
    Capability {
        capability: name,
        exposure: AgentOnly,
        ipc: None,
        mcp: Some(name),
    }
}

/// Every domain operation on either surface, classified. See module docs.
pub const MANIFEST: &[Capability] = &[
    // ---- both (identical names) ----
    both("ping"),
    both("app_version"),
    // Skills for agents that can't load skill files (ACP); the UI has no use for them.
    agent("get_skill"),
    // A dry run for an agent writing a source in its worktree.
    agent("preview_collector"),
    both("list_streams"),
    agent("get_task"),
    both("list_thread_notes"),
    agent("list_effort_observations"),
    // Per-effort metric roll-up for the task-page panel (tsk250): the
    // agent gets the same numbers as prompt text via oxplow-analytics'
    // `metric-deltas` advisory (over `v_effort_metric_delta`).
    model("list_efforts_in_window", &["v_effort"]),
    // Metrics read through SQL (`v_metric_spec`, `v_fact`, `metric_grid()`)
    // on both surfaces, and change through the `metric.*` commands
    // (`run_command`); nothing metric-specific is left on MCP (P4.8).
    ui(
        "enable_metrics",
        "the Catalog toggle, a person's typed call into `metric.enable`; an agent runs the command itself",
    ),
    // Architectural zones (tsk251) — agent-only: the agent reads the table
    // here and writes it with `config.set` (tsk411); the renderer only reads
    // it (it rides on `get_config`), so there is no IPC counterpart.
    agent("list_zones"),
    both("add_followup"),
    both("list_followups"),
    both("remove_followup"),
    both("list_backlinks"),
    both("list_outbound"),
    // ---- the command bus (agent side; the UI reaches commands through
    // lenses, the launcher and menus) ----
    agent("list_commands"),
    // The bus itself: an agent through MCP (as its verified thread), the
    // person through IPC (`Actor::Human`, confirming where asked).
    both("run_command"),
    ui(
        "get_command",
        "one command's spec, for a form or a confirmation; an agent lists them with `list_commands`",
    ),
    ui("undo_command", "undo is a person's (P5.A1)"),
    ui(
        "decide_proposal",
        "approving or declining an agent's proposal is a person's (P6b)",
    ),
    // ---- the event log's dead-letter queue ----
    both("list_dead_letters"),
    ui(
        "retry_dead_letter",
        "a dead letter's fate is a person's (V93); an agent lists them",
    ),
    ui(
        "discard_dead_letter",
        "a dead letter's fate is a person's (V93); an agent lists them",
    ),
    both("search"),
    both("query_sql"),
    // Settings → Data: the models, and entities not yet synced.
    ui(
        "list_data_entities",
        "the Data settings list, which joins the extensions' manifests: an agent reads published entities in `v_model` and one declared but not synced yet in its extension's `extension.yaml`",
    ),
    ui(
        "prompt_catalog",
        "what the person can ask oxplow's agent; the agent is who gets asked",
    ),
    ui(
        "effective_config",
        "the Settings view of every setting and its origin; an agent reads `config.list_keys`",
    ),
    ui("get_panel_layout", "the person's left-nav layout (P6.G1)"),
    ui("set_panel_layout", "the person's left-nav layout (P6.G1)"),
    both("list_extensions"),
    both("get_lens"),
    both("run_lens"),
    // A lens's actions: commands run as the lens, for whoever pressed.
    both("run_lens_action"),
    ui(
        "lens_text",
        "Copy's text on every lens; an agent reads the same text through `run_lens`",
    ),
    // An agent's answers: it shows one (the `lens.show` command).
    agent("show_lens"),
    ui(
        "run_answer",
        "runs an answer for the Answers strip; `show_lens` gives the agent its text",
    ),
    ui(
        "lens_form",
        "a form lens is the person's form; an agent runs the command itself",
    ),
    ui(
        "submit_lens_form",
        "a form lens is the person's form; an agent runs the command itself",
    ),
    ui(
        "run_component_query",
        "a custom component's bridged read, from its sandboxed frame through the host (P6b.D2)",
    ),
    ui(
        "load_component",
        "the host loads a custom component's bundle for the frame it shows the person (tsk984)",
    ),
    ui(
        "invoke_component_command",
        "a custom component's bridged command, from its sandboxed frame through the host (P6b.D2)",
    ),
    both("validate_extension"),
    both("review_extension"),
    ui(
        "report_open_page",
        "the person's open page, which an agent reads with `get_open_page`",
    ),
    ui(
        "set_extension_enabled",
        "turning extensions on or off is a person's call",
    ),
    both("list_collectors"),
    // The `collector.sync` command: the UI runs it through `run_command`,
    // an agent through this tool (as itself, so it never approves).
    agent("run_collector"),
    ui(
        "approve_collector",
        "consent to run a collector's program is a person's",
    ),
    ui(
        "set_credential",
        "secrets are a person's to set; an agent only sees whether one is set",
    ),
    ui(
        "list_project_programs",
        "the programs waiting on a person's consent (tsk331); an agent learns of an unapproved one from the run's error",
    ),
    ui(
        "approve_project_program",
        "consent to run a program from the repo is a person's (tsk331)",
    ),
    ui(
        "program_source",
        "a person reads what they're asked to approve; an agent reads its worktree's files",
    ),
    ui(
        "provider_declaration_effects",
        "what a provider's approval would change, shown before a person approves it (P6b.E3)",
    ),
    ui(
        "list_provider_instances",
        "Settings → Integrations, where a person configures instances (P5.D4, P9.B1)",
    ),
    ui(
        "check_provider_instance",
        "enabling a provider instance runs a program: a person's (P5.D4)",
    ),
    ui(
        "set_provider_instance",
        "enabling a provider instance runs a program: a person's (P5.D4)",
    ),
    ui(
        "add_provider_instance",
        "instances and their credentials are a person's (P9.B1)",
    ),
    ui(
        "remove_provider_instance",
        "instances and their credentials are a person's (P9.B1)",
    ),
    ui(
        "turn_off_provider_instance_here",
        "instances and their credentials are a person's (P9.B1)",
    ),
    ui(
        "set_instance_credential",
        "instances and their credentials are a person's (P9.B1)",
    ),
    ui(
        "begin_oauth_sign_in",
        "signing in is a person's, in their browser (P9.B3)",
    ),
    ui(
        "cancel_oauth_sign_in",
        "signing in is a person's, in their browser (P9.B3)",
    ),
    ui(
        "complete_oauth_sign_in",
        "signing in is a person's: the shell hands the core the redirect it caught (P10)",
    ),
    ui(
        "listen_for_oauth_redirect",
        "the desktop shell's loopback listener for a person's sign-in (P10)",
    ),
    ui(
        "await_oauth_redirect",
        "the desktop shell's loopback listener for a person's sign-in (P10)",
    ),
    ui(
        "answer_oauth_redirect",
        "the desktop shell's loopback listener for a person's sign-in (P10)",
    ),
    ui(
        "stop_oauth_redirect",
        "the desktop shell's loopback listener for a person's sign-in (P10)",
    ),
    both("ensure_change"),
    both_named("ai.settings", "ai_settings", "list_ai_roles"),
    ui(
        "save_ai_provider",
        "AI providers and their keys are a person's to set",
    ),
    ui(
        "remove_ai_provider",
        "AI providers and their keys are a person's to set",
    ),
    ui("set_ai_role", "AI settings are a person's to set"),
    ui(
        "test_ai_provider",
        "checks a person's AI provider settings before they save them",
    ),
    agent("ai_decide"),
    agent("ai_summarize"),
    agent("get_open_page"),
    agent("list_lenses"),
    // ---- both (names diverge across surfaces) ----
    both_named("thread.list", "list_threads", "list_thread_work"),
    agent("list_tasks"),
    both_named("comment.list", "list_comments_for_stream", "list_comments"),
    // ---- agent-only (orchestration / agent affordances) ----
    agent("read_task_options"),
    agent("dispatch_task"),
    agent("get_thread_context"),
    agent("await_user"),
    agent("wiki_ref_drift"),
    // ---- collection (effort-scoped observations) ----
    agent("get_open_effort"),
    agent("code_definition"),
    agent("code_hover"),
    agent("code_references"),
    agent("code_symbols"),
    agent("code_workspace_symbols"),
    agent("code_call_hierarchy"),
    agent("code_diagnostics"),
    agent("lsp_list_servers"),
    // ---- code analysis: generic per-language unit listing (tree-sitter) ----
    agent("list_code_units"),
    // ---- git: read tools mirrored to MCP ----
    both_named("git.status", "git_change_scopes", "git_status"),
    both("diff"),
    both("vcs_log"),
    both("vcs_blame"),
    both("read_at"),
    both("vcs_branches"),
    ui(
        "search_workspace_text",
        "an agent searches its worktree with its own Grep tool",
    ),
    // ---- snapshots / local history: reads + restore mirrored to MCP ----
    both("list_snapshots_for_stream"),
    both("list_snapshot_ops"),
    both("list_files_for_snapshot"),
    both("get_file_snapshot"),
    both("get_snapshot_stats"),
    agent("list_snapshot_change_entries"),
    agent("read_file_snapshot"),
    agent("read_file_at_snapshot"),
    both("read_event_content"),
    model("list_file_snapshots", &["v_snapshot_file"]),
    // ---- code quality: duplication findings mirrored to MCP ----
    agent("list_code_quality_findings"),
    // ---- UI selection mirrored to MCP ----
    both("select_thread"),
    both("switch_stream"),
    // ---- app / misc ----
    ui(
        "log_ui",
        "the renderer's console, forwarded to the daemon's log",
    ),
    // ---- streams ----
    model("get_primary_stream", &["v_stream"]),
    ui(
        "get_current_stream",
        "the stream the person has selected; an agent works in its own",
    ),
    // ---- threads ----
    ui(
        "list_acp_agents",
        "the thread picker's ACP agents (tsk335): an agent never starts an agent",
    ),
    // ACP sessions: the prompt box, permission cards and banners. Never
    // agent tools — an agent must not prompt an agent (tsk281).
    ui(
        "acp_open_session",
        "an ACP session is the person's: an agent must not prompt an agent (tsk281)",
    ),
    ui(
        "acp_prompt",
        "an ACP session is the person's: an agent must not prompt an agent (tsk281)",
    ),
    ui(
        "acp_cancel",
        "an ACP session is the person's: an agent must not prompt an agent (tsk281)",
    ),
    ui(
        "acp_respond_permission",
        "an ACP permission card is answered by a person, never an agent (tsk281)",
    ),
    ui(
        "acp_transcript",
        "an ACP session's transcript, rendered for the person (tsk281)",
    ),
    ui(
        "acp_dismiss_directive",
        "the person dismisses the turn-end banner (tsk281)",
    ),
    ui(
        "acp_close_session",
        "an ACP session is the person's: an agent must not prompt an agent (tsk281)",
    ),
    ui(
        "set_agents",
        "which agents a thread can run is a person's choice",
    ),
    model("list_closed_threads", &["v_thread"]),
    ui(
        "get_thread_state",
        "the person's selected and active thread; an agent reads threads in `v_thread` and has no selection",
    ),
    // ---- dashboards: reads + create/add-tile are agent-authorable ----
    both("list_dashboards"),
    both("get_dashboard"),
    // ---- comments (writes are knowledge.*) ----
    model("list_comments_for_target", &["v_comment"]),
    // ---- page visits ----
    ui(
        "record_page_visit",
        "records the pages the person opens, as they browse",
    ),
    model("list_recent_page_visits", &["v_page_visit"]),
    model("top_visited_pages", &["v_page_visit"]),
    ui(
        "forget_page",
        "forgetting a page they visited is the person's",
    ),
    // ---- usage ----
    ui(
        "record_usage",
        "records the person's use of oxplow, as they work",
    ),
    model("list_recent_usage_rollup", &["v_usage_event"]),
    // ---- snapshots (UI presentation helpers) ----
    model("list_wiki_slugs_for_snapshots", &["v_snapshot_file"]),
    ui(
        "get_blob_storage_bytes",
        "the blob store's size on disk, for Local History's storage card",
    ),
    // ---- git (presentation / worktree / remote helpers) ----
    model("git_resolve_commit_ref_labels", &["v_branch", "v_tag"]),
    model("git_list_recent_remote_branches", &["v_branch"]),
    ui(
        "vcs_list_adoptable_workspaces",
        "worktrees on disk a person may adopt as streams",
    ),
    ui(
        "vcs_divergence",
        "an ancestry walk the VCS answers; an agent runs git in its worktree",
    ),
    ui(
        "vcs_revisions_between",
        "an ancestry walk the VCS answers; an agent has `vcs_log`",
    ),
    model("vcs_file_history", &["v_commit_file"]),
    // ---- hooks / agent lifecycle ----
    ui(
        "ingest_hook_event",
        "the hook subprocess's transport into the core, not an agent's call",
    ),
    model("list_agent_events", &["v_event"]),
    ui(
        "list_agent_statuses",
        "agents' live status, shown to the person; an agent is the one with the status",
    ),
    model("list_open_agent_turns", &["v_agent_turn"]),
    model("get_agent_turn", &["v_agent_turn"]),
    // ---- config ----
    ui(
        "get_config",
        "the renderer's whole config; an agent reads keys with `config.list_keys`",
    ),
    ui(
        "set_agent_prompt_append",
        "the Settings form for a person-only key: an agent's `config.set` of it is a proposal the person decides",
    ),
    ui(
        "set_generated",
        "the Settings form; an agent sets config with `config.set`",
    ),
    ui(
        "set_agent_model",
        "the Settings form for a person-only key (`agentModels`): an agent's `config.set` of it is a proposal the person decides",
    ),
    ui(
        "get_workspace_context",
        "the window's project path and VCS state, for the shell",
    ),
    // ---- efforts ----
    model("get_effort_files", &["v_effort_file"]),
    model("get_effort", &["v_effort"]),
    model("list_efforts_at_snapshots", &["v_effort"]),
    model("list_efforts_overlapping_range", &["v_effort"]),
    // The files of the effort's snapshot bracket (`v_snapshot_file`
    // between `v_effort`'s start and end snapshots), split by whether the
    // effort claimed them (`v_effort_file`).
    model(
        "list_changed_paths_for_effort",
        &["v_effort", "v_snapshot_file", "v_effort_file"],
    ),
    // ---- workspace file I/O (agent uses Read/Write tools) ----
    ui(
        "list_workspace_entries",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "list_workspace_files",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "read_workspace_file",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "files_at",
        "a revision's file list, for the editor; an agent reads a revision with `read_at`",
    ),
    ui(
        "extension_effects_between",
        "an effort review's Extension Changes (P8.C7); an agent runs `oxplow plugin check --effects`",
    ),
    ui(
        "vcs_head",
        "the UI's live VCS state; an agent has `git_status`",
    ),
    ui(
        "vcs_status",
        "the UI's live VCS state; an agent has `git_status`",
    ),
    ui(
        "vcs_revision",
        "one revision's detail for the history view; an agent has `vcs_log`",
    ),
    ui(
        "vcs_merge_base",
        "an ancestry walk the VCS answers; an agent runs git in its worktree",
    ),
    ui(
        "write_workspace_file",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "create_workspace_file",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "create_workspace_directory",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "rename_workspace_path",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "delete_workspace_path",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    // ---- background tasks ----
    ui(
        "list_background_tasks",
        "the UI's tray of live background work",
    ),
    ui(
        "get_background_task",
        "the UI's tray of live background work",
    ),
    ui(
        "start_background_task",
        "the UI's tray of live background work",
    ),
    ui(
        "complete_background_task",
        "the UI's tray of live background work",
    ),
    ui(
        "fail_background_task",
        "the UI's tray of live background work",
    ),
    ui(
        "update_background_task",
        "the UI's tray of live background work",
    ),
    // ---- webview ----
    ui(
        "open_external_url",
        "a sandboxed native window: the shell's, never an agent's",
    ),
    ui(
        "clipboard_read_text",
        "the OS clipboard, for the person's paste",
    ),
    // ---- lsp (shared sessions + installer) ----
    ui(
        "list_installed_lsp_packages",
        "Settings' language servers; an agent has `lsp_list_servers`",
    ),
    ui(
        "lsp_request",
        "the editor's language-server client; an agent has the `code_*` tools",
    ),
    ui(
        "lsp_notify",
        "the editor's language-server client; an agent has the `code_*` tools",
    ),
    ui(
        "list_lsp_servers",
        "Settings' language servers; an agent has `lsp_list_servers`",
    ),
    ui(
        "restart_lsp_server",
        "Settings' language servers: restarting one is the person's",
    ),
    ui(
        "respond_lsp_apply_edit",
        "the editor answers a server's edit request",
    ),
    // ---- terminal ----
    // `forward_terminal_input` is UI-ONLY by design and must never reach
    // the MCP (agent) surface: it is the human keystroke/paste transport,
    // not an automation API — part of the no-automation guard (see
    // `.context/agent-model.md`).
    ui(
        "open_terminal_session",
        "the person's terminal: never an automation API",
    ),
    ui(
        "forward_terminal_input",
        "the human's keystroke and paste transport: oxplow never synthesizes agent input",
    ),
    ui(
        "close_terminal_session",
        "the person's terminal: never an automation API",
    ),
    ui(
        "terminate_terminal_session",
        "the person's terminal: never an automation API",
    ),
    ui(
        "terminal_session_cwd",
        "the person's terminal: never an automation API",
    ),
    ui(
        "lookup_terminal_session",
        "feeds `forward_terminal_input`, which is the human's alone",
    ),
    // ---- menu ----
    ui("set_native_menu", "the native menu bar"),
    // ---- launcher / multi-window ----
    ui("list_recent_projects", "the launcher's recent projects"),
    ui("remove_recent_project", "the launcher's recent projects"),
    ui("open_project", "the launcher opens a project window"),
    ui("create_project", "the launcher creates a project window"),
    ui("setup_project", "a person confirms a project's first-run setup"),
    ui("abort_setup", "a person declines a project's first-run setup"),
];

/// Validate the manifest's internal shape independent of the real surfaces:
/// per-exposure name presence, and uniqueness of capability/ipc/mcp names.
/// Returns a list of human-readable problems (empty == valid).
pub fn manifest_shape_errors() -> Vec<String> {
    use std::collections::HashSet;
    let mut errs = Vec::new();
    let mut caps = HashSet::new();
    let mut ipcs = HashSet::new();
    let mut mcps = HashSet::new();
    for c in MANIFEST {
        if !caps.insert(c.capability) {
            errs.push(format!("duplicate capability key: {}", c.capability));
        }
        if let Some(name) = c.ipc {
            if !ipcs.insert(name) {
                errs.push(format!("duplicate ipc name: {name}"));
            }
        }
        if let Some(name) = c.mcp {
            if !mcps.insert(name) {
                errs.push(format!("duplicate mcp name: {name}"));
            }
        }
        let ok = match c.exposure {
            Both => c.ipc.is_some() && c.mcp.is_some(),
            UiOnly { .. } | Model { .. } => c.ipc.is_some() && c.mcp.is_none(),
            AgentOnly => c.ipc.is_none() && c.mcp.is_some(),
        };
        if !ok {
            errs.push(format!(
                "{} ({:?}) has invalid ipc/mcp name combination: ipc={:?} mcp={:?}",
                c.capability, c.exposure, c.ipc, c.mcp
            ));
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_shape_is_valid() {
        let errs = manifest_shape_errors();
        assert!(
            errs.is_empty(),
            "manifest shape errors:\n{}",
            errs.join("\n")
        );
    }

    /// P11 (tsk944): a UI-only row says why an agent has no tool for it.
    #[test]
    fn every_ui_only_row_says_why() {
        let silent: Vec<&str> = MANIFEST
            .iter()
            .filter(|c| matches!(c.exposure, UiOnly { why } if why.trim().len() < 12))
            .map(|c| c.capability)
            .collect();
        assert!(
            silent.is_empty(),
            "UI-only rows that don't say why: {silent:?}"
        );
    }

    /// P4.8 (tsk493): metrics are read through SQL and changed through
    /// `metric.*` commands, so no surface carries a metric-specific read.
    #[test]
    fn no_surface_carries_a_metric_read() {
        let reads = [
            "metric_series",
            "metric_rollup",
            "metric_breakdown",
            "list_metric_samples",
            "list_metric_definitions",
            "list_metric_findings",
            "list_metric_catalog",
            "get_metric_summary",
            "list_measures",
            "list_dimensions",
            "list_facts",
        ];
        for c in MANIFEST {
            for name in [Some(c.capability), c.ipc, c.mcp].into_iter().flatten() {
                assert!(!reads.contains(&name), "`{name}` is a metric read");
            }
        }
    }
}
