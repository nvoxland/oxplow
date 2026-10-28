//! Cross-surface parity manifest for oxplow's two adapter layers.
//!
//! Oxplow exposes domain operations on two independent adapters, both thin
//! wrappers over `oxplow_app::Services`:
//!   - **Tauri IPC** (`oxplow-tauri-ipc`) — `#[tauri::command]` fns the React
//!     UI calls.
//!   - **MCP** (`oxplow-mcp`) — the tools the agent calls: core's
//!     `list_commands` / `run_command` and the ones oxplow's extensions
//!     declare (`tool:` on a command or a page, `oxplow_app::agent_tools`).
//!
//! These drifted silently: many user-meaningful ops lived on IPC but not MCP.
//! This manifest is the single source of truth for *which* surface each
//! capability should live on, and `tests/parity.rs` enforces that the real
//! registrations match it. Adding a `#[tauri::command]` or a `tool:` without a
//! matching row fails the test, forcing an explicit classification.
//!
//! ## The four exposures
//! - [`Exposure::Both`] — present on IPC and MCP (names may diverge per
//!   surface), with [`One`] saying how the two stay one operation: the
//!   IPC adapter runs the agent's bus command as the person
//!   ([`One::Bus`], which `tests/parity.rs` checks in the adapter's
//!   source), or the person's shape differs from the agent's and both
//!   call the same service ([`One::Shape`], saying why).
//! - [`Exposure::UiOnly`] — intentionally UI-only, saying why an agent has
//!   no tool for it: a person's consent or setting, the person's own
//!   selection or input, live state pushed to the UI, runtime infra (menus,
//!   terminals, the LSP client, windows), an ancestry walk the VCS answers,
//!   or workspace file I/O the agent does with its own Read/Write tools.
//! - [`Exposure::Dataset`] — a UI read whose agent counterpart is published
//!   datasets: an agent reads the same rows with `query_sql`. Each dataset
//!   named must be published (`tests/parity.rs`).
//! - [`Exposure::AgentOnly`] — intentionally agent-only (batch and
//!   orchestration affordances).
//!
//! Every row is decided: there is no "build the MCP tool later" exposure.
//! An agent's way to an IPC read is a tool (`Both`), a dataset (`Dataset`), or
//! none, with the reason (`UiOnly`).

/// Which adapter surface(s) a capability is expected to live on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Live on both the IPC (UI) and MCP (agent) surfaces, one operation.
    Both { one: One },
    /// Intentionally UI-only: `why` an agent has no tool for it.
    UiOnly { why: &'static str },
    /// A UI read an agent makes through these published datasets instead
    /// (`query_sql`).
    Dataset { datasets: &'static [&'static str] },
    /// Intentionally agent-only.
    AgentOnly,
}

/// How a [`Exposure::Both`] row is one operation on two front doors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum One {
    /// The IPC adapter runs this bus command as the person, so the bus
    /// validates and answers for both.
    Bus { command: &'static str },
    /// The person's call has another shape (an open run, a UI-only
    /// switch); both sides call the same service. `why` says what differs.
    Shape { why: &'static str },
}

/// One domain capability and the name it carries on each surface.
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    /// Stable human label — the parity row key. Must be unique.
    pub capability: &'static str,
    pub exposure: Exposure,
    /// IPC command name, or `None` when the capability is agent-only.
    pub ipc: Option<&'static str>,
    /// MCP tool name, or `None` when UI-only or read through datasets.
    pub mcp: Option<&'static str>,
}

use Exposure::*;

/// A `Both` row whose IPC adapter runs the agent's bus `command`.
const fn via_bus(
    capability: &'static str,
    ipc: &'static str,
    mcp: &'static str,
    command: &'static str,
) -> Capability {
    Capability {
        capability,
        exposure: Both {
            one: One::Bus { command },
        },
        ipc: Some(ipc),
        mcp: Some(mcp),
    }
}
/// A `Both` row whose person's call has another shape; `why`.
const fn both_shaped(
    capability: &'static str,
    ipc: &'static str,
    mcp: &'static str,
    why: &'static str,
) -> Capability {
    Capability {
        capability,
        exposure: Both {
            one: One::Shape { why },
        },
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
/// Helper for a UI read whose agent counterpart is published datasets.
const fn dataset(name: &'static str, datasets: &'static [&'static str]) -> Capability {
    Capability {
        capability: name,
        exposure: Dataset { datasets },
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
    dataset("list_streams", &["v_stream"]),
    // The work-item interface, read for the agent; the UI reads the
    // datasets (`v_work_item`).
    // Per-effort metric roll-up for the task-page panel (tsk250): the
    // agent gets the same numbers as prompt text via oxplow-bundled's
    // `metric-deltas` advisory (over `v_effort_metric_delta`).
    dataset("list_efforts_in_window", &["v_effort"]),
    // Metrics read through SQL (`v_metric_spec`, `v_fact`, `metric_grid()`)
    // on both surfaces, and change through the `metric.*` commands
    // (`run_command`); nothing metric-specific is left on MCP (P4.8).
    // Architectural zones (tsk251) — agent-only: the agent reads the table
    // here and writes it with `oxplow.config.set` (tsk411); the renderer only reads
    // it (it rides on `get_config`), so there is no IPC counterpart.
    dataset("list_backlinks", &["v_page_ref"]),
    dataset("list_outbound", &["v_page_ref"]),
    // ---- the command bus (agent side; the UI reaches commands through
    // pages, the launcher and menus) ----
    agent("list_commands"),
    // The services an extension's script may use (`oxplow_domain::service`):
    // Settings and the reference show a person the table; an agent writing
    // an extension reads the scopes from `v_scope`.
    dataset("list_services", &["v_scope"]),
    // The bus itself: an agent through MCP (as its verified thread), the
    // person through IPC (`Actor::Human`, confirming where asked).
    both_shaped(
        "run_command",
        "run_command",
        "run_command",
        "the bus itself: any command, run as the caller",
    ),
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
    via_bus("search", "search", "oxplow_search", "oxplow.search.find"),
    both_shaped(
        "query_sql",
        "query_sql",
        "oxplow_query_sql",
        "the explorer's `raw` switch reads tables, which no agent may; both read through the one gateway",
    ),
    ui("get_panel_layout", "the person's left-nav layout (P6.G1)"),
    ui("set_panel_layout", "the person's left-nav layout (P6.G1)"),
    dataset("get_ui_preferences", &["v_ui_preference"]),
    ui("set_ui_preference", "the person's page memory and app preferences, a UI pointer like the panel layout"),
    dataset("list_extensions", &["v_extension", "v_contribution"]),
    // A window's open page runs: it opens one, hears which loaders
    // changed and fetches them. An agent runs a page once (`run_page`).
    ui("open_page", "a window keeps a page run open to hear its changes"),
    ui("close_page", "a window drops a page run it no longer shows"),
    ui("set_page_params", "a pick, filter or switch in the page a window shows"),
    ui("rerun_page", "Refresh on the page a window shows"),
    ui("fetch_page_runs", "a window fetches the open runs it heard changed"),
    both_shaped(
        "run_page",
        "run_page",
        "oxplow_run_page",
        "a window draws the run's tree; an agent reads its text, a draft or one loader; both run the one PageRunner",
    ),
    // An agent finds the pages that answer a question; a person has the
    // launcher and the catalog page.
    agent("oxplow_find_page"),
    // An agent shows the person a page (or one of its own); the person
    // opens pages themselves.
    agent("oxplow_show_page"),
    ui("page_text", "Copy's text on the page a window shows"),
    ui("get_page", "the page a window draws; an agent reads a page's ref with `read`"),
    ui("page_spec", "the source YAML a window edits; agents read page definitions through `read`"),
    both_shaped(
        "run_page_action",
        "run_page_action",
        "oxplow_run_page_action",
        "a person presses a button on a run a window holds, and confirms; an agent opens a run and never confirms; both press through pages::actions",
    ),
    ui("page_form", "a form node is the person's form; an agent runs the command itself"),
    ui("submit_page_form", "a form node is the person's form; an agent runs the command itself"),
    ui("page_action_form", "a form action's form is the person's; an agent runs the command itself"),
    ui(
        "load_component",
        "the host loads a custom component's bundle for the frame it shows the person (tsk984)",
    ),
    ui(
        "invoke_component_command",
        "a custom component's bridged command, from its sandboxed frame through the host (P6b.D2)",
    ),
    ui(
        "run_component_sql",
        "a custom component's own read-only SQL, from its sandboxed frame through the host; an agent runs oxplow_query_sql",
    ),
    ui(
        "component_form",
        "the form of a custom component's command, which the host shows the person when its frame asks",
    ),
    agent("oxplow_validate_extension"),
    ui(
        "report_tabs",
        "the tabs a window shows, which an agent reads with `list_tabs` and `get_open_page`",
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
    agent("oxplow_ensure_change"),
    dataset("ai_settings", &["v_ai_role", "v_ai_provider"]),
    ui("read_ai_call", "a person's look at a recorded model call; an agent reads its own with `read`"),
    ui("assistant_open", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_prompt", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_cancel", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_set", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_answer", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_snapshot", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_handoff", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    ui("assistant_close", "the Assistant's panel: a person's own conversation with oxplow's in-app agent; an agent has its own session"),
    agent("oxplow_ai_decide"),
    agent("oxplow_ai_summarize"),
    agent("oxplow_get_open_page"),
    agent("oxplow_list_tabs"),
    // ---- both (names diverge across surfaces) ----
    dataset("list_threads", &["v_thread"]),
    both_shaped(
        "comment.list",
        "list_comments_for_stream",
        "oxplow_list_comments",
        "the comment layer draws a stream's threads itself; an agent's read filters by status and resolves what each names; both read the one store",
    ),
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
    agent("oxplow_effort_summary"),
    agent("oxplow_catalog"),
    // ---- git ----
    agent("oxplow_vcs_status"),
    agent("oxplow_diff"),
    agent("oxplow_vcs_log"),
    via_bus("vcs_blame", "vcs_blame", "oxplow_vcs_blame", "oxplow.vcs.blame"),
    ui("read_at", "the editor's file at a revision; an agent reads a file ref at that revision with `read`"),
    ui(
        "resolve_symbol",
        "opening a symbol link; an agent reads v_symbol, and `read` for a pinned version",
    ),
    agent("oxplow_vcs_branches"),
    // ---- snapshots / local history ----
    dataset("list_snapshot_ops", &["v_snapshot_op"]),
    dataset("list_files_for_snapshot", &["v_snapshot_file"]),
    dataset("get_file_snapshot", &["v_snapshot_file"]),
    dataset("get_snapshot_stats", &["v_snapshot_file"]),
    ui("read_event_content", "the event page's stored body; an agent reads the event's ref with `read`"),
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
    dataset("get_primary_stream", &["v_stream"]),
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
        "get_thread_state",
        "the person's selected and active thread; an agent reads threads in `v_thread` and has no selection",
    ),
    // ---- comments (writes are knowledge.*) ----
    dataset("list_comments_for_target", &["v_comment"]),
    // ---- page visits ----
    ui(
        "record_page_visit",
        "records the pages the person opens, as they browse",
    ),
    dataset("list_recent_page_visits", &["v_page_visit"]),
    dataset("top_visited_pages", &["v_page_visit"]),
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
    dataset("list_wiki_slugs_for_snapshots", &["v_snapshot_file"]),
    ui(
        "get_blob_storage_bytes",
        "the blob store's size on disk, for Local History's storage card",
    ),
    // ---- git (presentation / worktree / remote helpers) ----
    dataset("git_resolve_commit_ref_labels", &["v_branch", "v_tag"]),
    // ---- hooks / agent lifecycle ----
    ui(
        "ingest_hook_event",
        "the hook subprocess's transport into the core, not an agent's call",
    ),
    ui(
        "list_agent_statuses",
        "agents' live status, shown to the person; an agent is the one with the status",
    ),
    // ---- config ----
    ui(
        "get_config",
        "the renderer's whole config; an agent reads keys with `oxplow.config.list_keys`",
    ),
    ui(
        "get_workspace_context",
        "the window's project path and VCS state, for the shell",
    ),
    // ---- efforts ----
    dataset("list_efforts_at_snapshots", &["v_effort"]),
    // ---- workspace file I/O (agent uses Read/Write tools) ----
    ui(
        "read_workspace_file",
        "workspace file I/O: an agent uses its own Read/Write tools",
    ),
    ui(
        "files_at",
        "a revision's file list, for the editor; an agent reads a revision's file with `read`",
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
    dataset("list_lsp_servers", &["v_lsp_server"]),
    ui(
        "respond_lsp_apply_edit",
        "the editor answers a server's edit request",
    ),
    // ---- terminal ----
    // The person's terminal is UI-ONLY by design and must never reach the
    // MCP (agent) surface. Its keystrokes aren't a command at all: they
    // ride the `/events` socket (`terminal_input`), never an automation
    // API — part of the no-automation guard (see `.context/agent-model.md`).
    ui(
        "open_terminal_session",
        "the person's terminal: never an automation API",
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
        "the person's terminal: never an automation API",
    ),
    // ---- menu ----
    ui("set_native_menu", "the native menu bar"),
    // ---- launcher / multi-window ----
    ui("list_recent_projects", "the launcher's recent projects"),
    ui("remove_recent_project", "the launcher's recent projects"),
    ui("open_project", "the launcher opens a project window"),
    ui("create_project", "the launcher creates a project window"),
    ui("setup_project", "a person confirms a project's first-run setup"),
    ui("install_cli", "a person puts the oxplow command on their PATH"),
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
            Both { .. } => c.ipc.is_some() && c.mcp.is_some(),
            UiOnly { .. } | Dataset { .. } => c.ipc.is_some() && c.mcp.is_none(),
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
