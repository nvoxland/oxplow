//! Cross-surface parity manifest for oxplow's two adapter layers.
//!
//! Oxplow exposes domain operations on two independent adapters, both thin
//! wrappers over `oxplow_app::Services`:
//!   - **Tauri IPC** (`oxplow-tauri-ipc`) — `#[tauri::command]` fns the React
//!     UI calls.
//!   - **MCP** (`oxplow-mcp`) — the tools the agent calls: core's
//!     `list_commands` / `run_command` and the ones oxplow's extensions
//!     declare (`tool:` on a command or a lens, `oxplow_app::agent_tools`).
//!
//! These drifted silently: many user-meaningful ops lived on IPC but not MCP.
//! This manifest is the single source of truth for *which* surface each
//! capability should live on, and `tests/parity.rs` enforces that the real
//! registrations match it. Adding a `#[tauri::command]` or a `tool:` without a
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
//! - [`Exposure::AgentOnly`] — intentionally agent-only (batch and
//!   orchestration affordances).
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
    ui(
        "ping",
        "the UI's health check of its backend; an agent's connection is its own liveness",
    ),
    ui(
        "app_version",
        "the version the About box shows",
    ),
    // Skills for agents that can't load skill files (ACP); the UI has no use for them.
    agent("oxplow_get_skill"),
    // A dry run for an agent writing a source in its worktree.
    agent("oxplow_preview_collector"),
    // A whole extension in a throwaway oxplow, for an agent authoring it;
    // a person runs `oxplow extension test`.
    agent("oxplow_preview_extension"),
    model("list_streams", &["v_stream"]),
    // The work-item interface, read for the agent; the UI reads the
    // models (`v_work_item`).
    model("list_thread_notes", &["v_thread_note"]),
    // Per-effort metric roll-up for the task-page panel (tsk250): the
    // agent gets the same numbers as prompt text via oxplow-bundled's
    // `metric-deltas` advisory (over `v_effort_metric_delta`).
    model("list_efforts_in_window", &["v_effort"]),
    // Metrics read through SQL (`v_metric_spec`, `v_fact`, `metric_grid()`)
    // on both surfaces, and change through the `metric.*` commands
    // (`run_command`); nothing metric-specific is left on MCP (P4.8).
    ui(
        "enable_metrics",
        "the Catalog toggle, a person's typed call into `oxplow.metric.enable`; an agent runs the command itself",
    ),
    // Architectural zones (tsk251) — agent-only: the agent reads the table
    // here and writes it with `oxplow.config.set` (tsk411); the renderer only reads
    // it (it rides on `get_config`), so there is no IPC counterpart.
    model("list_backlinks", &["v_page_ref"]),
    model("list_outbound", &["v_page_ref"]),
    // ---- the command bus (agent side; the UI reaches commands through
    // lenses, the launcher and menus) ----
    agent("list_commands"),
    // The services an extension's script may use (`oxplow_domain::service`):
    // Settings and the reference show a person the table; an agent writing
    // an extension reads the scopes from `v_scope`.
    model("list_services", &["v_scope"]),
    // The bus itself: an agent through MCP (as its verified thread), the
    // person through IPC (`Actor::Human`, confirming where asked).
    both("run_command"),
    ui(
        "command_open_after",
        "binds the page a person's run opens next (`ui.open_after`), for the window",
    ),
    ui(
        "get_command",
        "one command's spec, for a form or a confirmation; an agent lists them with `list_commands`",
    ),
    ui(
        "list_person_commands",
        "the commands a person is offered (their `ui`); an agent lists all it may run with `list_commands`",
    ),
    ui(
        "register_client_host",
        "the window says it's open and which scopes it hosts; an agent reaches them by running their commands",
    ),
    ui(
        "unregister_client_host",
        "the window says it's closing; an agent's calls to it are refused from then on",
    ),
    ui(
        "answer_client_call",
        "the window answers a call to a scope it hosts; an agent's call is answered through `run_command`",
    ),
    ui(
        "run_command_for_call",
        "the window runs a command while answering an agent's call, as that agent; an agent runs commands through `run_command`",
    ),
    ui("undo_command", "undo is a person's (P5.A1)"),
    ui(
        "decide_proposal",
        "approving or declining an agent's proposal is a person's (P6b)",
    ),
    // ---- the event log's dead-letter queue ----
    model("list_dead_letters", &["v_event_dead_letter"]),
    ui(
        "retry_dead_letter",
        "a dead letter's fate is a person's (V93); an agent lists them",
    ),
    ui(
        "discard_dead_letter",
        "a dead letter's fate is a person's (V93); an agent lists them",
    ),
    both_named("search", "search", "oxplow_search"),
    both_named("query_sql", "query_sql", "oxplow_query_sql"),
    // Settings → Data: the models, and entities not yet synced.
    ui(
        "list_data_entities",
        "the Data settings list, which joins the extensions' manifests: an agent reads published entities in `v_model` and one declared but not synced yet in its extension's `extension.yaml`",
    ),
    ui("get_panel_layout", "the person's left-nav layout (P6.G1)"),
    ui("set_panel_layout", "the person's left-nav layout (P6.G1)"),
    model("list_extensions", &["v_extension", "v_contribution"]),
    ui("get_lens", "the lens page's definition; an agent reads the lens's ref with `read`"),
    both_named("run_lens", "run_lens", "oxplow_run_lens"),
    // A lens's actions: commands run as the lens, for whoever pressed.
    both_named("run_lens_action", "run_lens_action", "oxplow_run_lens_action"),
    ui(
        "lens_text",
        "Copy's text on every lens; an agent reads the same text through `run_lens`",
    ),
    // An agent's answers: it shows one (the `oxplow.lens.show` command).
    agent("oxplow_show_lens"),
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
        "lens_action_form",
        "a form action's form is the person's; an agent runs the command itself",
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
    both_named("validate_extension", "validate_extension", "oxplow_validate_extension"),
    ui(
        "list_native_conversations",
        "a harness's saved conversations, for the person choosing one to resume or fork (their selection)",
    ),
    ui(
        "list_earlier_sessions",
        "a thread's earlier sessions and saved conversations, for the person choosing one to bring back (their selection)",
    ),
    ui(
        "report_tabs",
        "the tabs a window shows, which an agent reads with `list_tabs` and `get_open_page`",
    ),
    model(
        "list_collectors",
        &["v_collector_state", "v_collector_credential", "v_collector_run"],
    ),
    // The `oxplow.collector.sync` command: the UI runs it through `run_command`,
    // an agent through this tool (as itself, so it never approves).
    ui(
        "approve_collector",
        "consent to run a collector's program is a person's",
    ),
    ui(
        "revoke_collector",
        "consent is a person's to take back, as it is to give",
    ),
    ui(
        "set_credential",
        "secrets are a person's to set; an agent only sees whether one is set",
    ),
    ui(
        "save_extension_secret",
        "a host secret reference's value is a person's to set; no agent route reads or writes one",
    ),
    ui(
        "extension_secret_is_set",
        "presence of a host secret, for the settings row; an agent never needs it",
    ),
    ui(
        "revoke_extension_secret",
        "a person takes a host secret back, as they gave it",
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
        "revoke_project_program",
        "consent is a person's to take back, as it is to give",
    ),
    ui(
        "program_source",
        "a person reads what they're asked to approve; an agent reads its worktree's files",
    ),
    ui(
        "provider_declaration_impact",
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
    both_named("ensure_change", "ensure_change", "oxplow_ensure_change"),
    model("ai_settings", &["v_ai_role", "v_ai_provider"]),
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
    ui("read_ai_call", "a person's look at a recorded model call; an agent reads its own with `read`"),
    agent("oxplow_ai_decide"),
    agent("oxplow_ai_summarize"),
    agent("oxplow_get_open_page"),
    agent("oxplow_list_tabs"),
    // ---- both (names diverge across surfaces) ----
    model("list_threads", &["v_thread"]),
    both_named("comment.list", "list_comments_for_stream", "oxplow_list_comments"),
    // ---- agent-only (orchestration / agent affordances) ----
    // One reader for anything a ref names.
    agent("oxplow_read"),
    agent("oxplow_wiki_ref_drift"),
    // ---- collection (effort-scoped observations) ----
    agent("oxplow_code_definition"),
    agent("oxplow_code_hover"),
    agent("oxplow_code_references"),
    agent("oxplow_code_symbols"),
    agent("oxplow_code_workspace_symbols"),
    agent("oxplow_code_call_hierarchy"),
    agent("oxplow_navigate"),
    agent("oxplow_thread_work"),
    agent("oxplow_work_next"),
    agent("oxplow_effort_files"),
    agent("oxplow_effort_summary"),
    agent("oxplow_file_history"),
    agent("oxplow_recent_commits"),
    agent("oxplow_catalog"),
    agent("oxplow_snapshots"),
    agent("oxplow_session_transcript"),
    agent("oxplow_thread_notes"),
    agent("oxplow_thread_sessions"),
    // ---- git ----
    both_named("git.status", "git_change_scopes", "oxplow_vcs_status"),
    both_named("diff", "diff", "oxplow_diff"),
    ui(
        "changed_paths",
        "an agent reads the same paths, with line counts, through `diff`",
    ),
    both_named("vcs_log", "vcs_log", "oxplow_vcs_log"),
    both_named("vcs_blame", "vcs_blame", "oxplow_vcs_blame"),
    ui("read_at", "the editor's file at a revision; an agent reads a file ref at that revision with `read`"),
    ui(
        "resolve_symbol",
        "opening a symbol link; an agent reads v_symbol, and `read` for a pinned version",
    ),
    both_named("vcs_branches", "vcs_branches", "oxplow_vcs_branches"),
    ui(
        "search_workspace_text",
        "an agent searches its worktree with its own Grep tool",
    ),
    // ---- snapshots / local history ----
    model("list_snapshots_for_stream", &["v_snapshot"]),
    model("list_snapshot_ops", &["v_snapshot_op"]),
    model("list_files_for_snapshot", &["v_snapshot_file"]),
    model("get_file_snapshot", &["v_snapshot_file"]),
    model("get_snapshot_stats", &["v_snapshot_file"]),
    ui("read_event_content", "the event page's stored body; an agent reads the event's ref with `read`"),
    model("list_file_snapshots", &["v_snapshot_file"]),
    // ---- UI selection ----
    ui(
        "select_thread",
        "the person's selection; an agent's thread is its session's",
    ),
    ui(
        "switch_stream",
        "the person's selection; an agent's stream is its thread's",
    ),
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
        "list_agent_harnesses",
        "the session picker's and Settings' harnesses: an agent never starts an agent",
    ),
    ui(
        "set_agents",
        "which agents a thread can run is a person's choice",
    ),
    ui(
        "get_thread_state",
        "the person's selected and active thread; an agent reads threads in `v_thread` and has no selection",
    ),
    // ---- dashboards: agents read the models; writes are `dashboard.*` commands ----
    model("list_dashboards", &["v_dashboard"]),
    model("get_dashboard", &["v_dashboard", "v_dashboard_item"]),
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
        "an ancestry walk the VCS answers; an agent reads `v_commit`",
    ),
    model("vcs_file_history", &["v_commit_file"]),
    // ---- hooks / agent lifecycle ----
    ui(
        "ingest_hook_event",
        "the hook subprocess's transport into the core, not an agent's call",
    ),
    ui(
        "list_agent_statuses",
        "agents' live status, shown to the person; an agent is the one with the status",
    ),
    model("list_open_agent_turns", &["v_agent_turn"]),
    model("get_agent_turn", &["v_agent_turn"]),
    // ---- config ----
    ui(
        "get_config",
        "the renderer's whole config; an agent reads keys with `oxplow.config.list_keys`",
    ),
    ui(
        "set_agent_prompt_append",
        "the Settings form for a person-only key: an agent's `oxplow.config.set` of it is a proposal the person decides",
    ),
    ui(
        "set_generated",
        "the Settings form; an agent sets config with `oxplow.config.set`",
    ),
    ui(
        "set_agent_setting",
        "the Settings form for a person-only key (`agentConfig`): an agent's `oxplow.config.set` of it is a proposal the person decides",
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
        "a revision's file list, for the editor; an agent reads a revision's file with `read`",
    ),
    ui(
        "extension_impact_between",
        "an effort review's Extension Changes; an agent runs `oxplow extension check --impact`",
    ),
    ui(
        "vcs_revision",
        "one revision's detail for the history view; an agent reads `v_commit`",
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
        "Settings' language servers; an agent reads `v_lsp_server`",
    ),
    ui(
        "lsp_request",
        "the editor's language-server client; an agent has the `code_*` tools",
    ),
    ui(
        "lsp_notify",
        "the editor's language-server client; an agent has the `code_*` tools",
    ),
    model("list_lsp_servers", &["v_lsp_server"]),
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
        "rename_native_conversation",
        "the person's rename of a session's conversation in its harness's own UI: never an automation API",
    ),
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
