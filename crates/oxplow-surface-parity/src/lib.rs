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
//! - [`Exposure::UiOnly`] — intentionally UI-only (Tauri/runtime infra:
//!   menus, terminals, LSP-client lifecycle, telemetry, background tasks,
//!   launcher, workspace file I/O the agent does via its own Read/Write tools).
//! - [`Exposure::AgentOnly`] — intentionally agent-only (dispatch, await_user,
//!   delegate_query, batch/orchestration affordances).
//! - [`Exposure::AgentTodo`] — *should* be on both; the MCP tool is not built
//!   yet. A tracked, reviewed gap. `ipc` is set, `mcp` is `None`.
//!
//! ## The ratchet (closing a gap)
//! When you build the MCP tool for an `AgentTodo` row, flip its `exposure` to
//! `Both` and fill in `mcp: Some("<new_tool>")`. If you forget, the parity
//! test's "every registered tool is classified" check fails on the new
//! unclaimed tool — so drift is caught from both directions, in one diff.

/// Which adapter surface(s) a capability is expected to live on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Live on both the IPC (UI) and MCP (agent) surfaces.
    Both,
    /// Intentionally UI-only.
    UiOnly,
    /// Intentionally agent-only.
    AgentOnly,
    /// Intended for both; MCP tool not built yet. Tracked gap.
    AgentTodo,
}

/// One domain capability and the name it carries on each surface.
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    /// Stable human label — the parity row key. Must be unique.
    pub capability: &'static str,
    pub exposure: Exposure,
    /// IPC command name, or `None` when the capability is agent-only.
    pub ipc: Option<&'static str>,
    /// MCP tool name, or `None` when ui-only or not-yet-built (`AgentTodo`).
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
/// Helper for an intentionally UI-only command.
const fn ui(name: &'static str) -> Capability {
    Capability {
        capability: name,
        exposure: UiOnly,
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
/// Helper for a tracked gap: IPC exists, MCP tool not built yet.
const fn todo(name: &'static str) -> Capability {
    Capability {
        capability: name,
        exposure: AgentTodo,
        ipc: Some(name),
        mcp: None,
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
    agent("preview_source"),
    both("list_streams"),
    agent("get_task"),
    agent("create_task"),
    agent("update_task"),
    agent("upsert_task"),
    agent("reorder_tasks"),
    both("add_thread_note"),
    both("list_thread_notes"),
    agent("list_effort_observations"),
    // Per-effort metric roll-up for the task-page panel (tsk250) — UI-only; the
    // agent gets the same numbers as prompt text via oxplow-analytics'
    // `metric-deltas` advisory (over `v_effort_metric_delta`).
    // Effort bands on the Metrics Explorer time axis (tsk233) — UI-only overlay.
    ui("list_efforts_in_window"),
    // Metrics read through SQL (`v_metric_spec`, `v_fact`, `metric_grid()`)
    // on both surfaces, and change through the `metric.*` commands
    // (`run_command`); nothing metric-specific is left on MCP (P4.8).
    // The catalog toggle (tsk219): the person's typed call into the
    // `metric.enable` command; an agent runs the command itself.
    ui("enable_metrics"),
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
    // person through IPC (`Actor::Human`, confirming where asked). Undo is
    // the person's (P5.A1).
    both("run_command"),
    // One command's spec, for a form or a confirmation; agents list them
    // with list_commands.
    ui("get_command"),
    ui("undo_command"),
    // Approving or declining an agent's proposal is a person's (P6b).
    ui("decide_proposal"),
    // ---- the event log's dead-letter queue ----
    both("list_dead_letters"),
    // A person decides a dead letter's fate (V93): agents can list them.
    ui("retry_dead_letter"),
    ui("discard_dead_letter"),
    both("search"),
    both("query_sql"),
    // Settings → Data (models with counts, unsynced entities); an agent
    // reads v_model and counts with query_sql.
    ui("list_data_entities"),
    ui("prompt_catalog"),
    ui("effective_config"),
    ui("get_panel_layout"),
    ui("set_panel_layout"),
    both("list_extensions"),
    both("get_lens"),
    both("run_lens"),
    // A lens's actions: commands run as the lens, for whoever pressed.
    both("run_lens_action"),
    // Copy on every lens; an agent reads the same text through run_lens.
    ui("lens_text"),
    // An agent's answers: it shows one (the `lens.show` command); the UI
    // runs each for the Answers strip.
    agent("show_lens"),
    ui("run_answer"),
    // A form lens: agents run the command itself.
    ui("lens_form"),
    ui("submit_lens_form"),
    both("validate_extension"),
    both("review_extension"),
    both("install_extension"),
    both("update_extension"),
    ui("save_lens"),
    ui("report_open_page"),
    // Turning extensions on/off is the person's call.
    ui("set_extension_enabled"),
    both("list_sources"),
    agent("record_decision"),
    agent("record_claim"),
    // The `source.sync` command: the UI runs it through `run_command`,
    // an agent through this tool (as itself, so it never approves).
    agent("run_source"),
    // Consent to run a source's program is a person's.
    ui("approve_source"),
    // Secrets are the person's to set; agents only see whether one is set.
    ui("set_source_credential"),
    // Consent to run a program from the repo is a person's (tsk331); an
    // agent learns an unapproved one from the run's error.
    ui("list_project_programs"),
    ui("approve_project_program"),
    // Enabling a provider instance runs a program: a person's (P5.D4).
    ui("list_provider_instances"),
    ui("check_provider_instance"),
    ui("set_provider_instance"),
    both("ensure_change"),
    both_named("ai.settings", "ai_settings", "list_ai_roles"),
    ui("save_ai_provider"),
    ui("remove_ai_provider"),
    ui("set_ai_role"),
    ui("test_ai_provider"),
    agent("ai_decide"),
    agent("ai_summarize"),
    agent("get_open_page"),
    agent("list_lenses"),
    // ---- both (names diverge across surfaces) ----
    both_named("thread.list", "list_threads", "list_thread_work"),
    agent("list_tasks"),
    both_named("comment.list", "list_comments_for_stream", "list_comments"),
    both_named(
        "comment.respond",
        "add_comment_message",
        "respond_to_comment",
    ),
    both_named(
        "comment.set_status",
        "set_comment_status",
        "resolve_comment",
    ),
    // ---- agent-only (orchestration / agent affordances) ----
    agent("read_task_options"),
    agent("complete_task"),
    agent("amend_effort"),
    agent("transition_tasks"),
    agent("dispatch_task"),
    agent("get_thread_context"),
    agent("file_epic_with_children"),
    agent("delegate_query"),
    agent("record_query_finding"),
    agent("await_user"),
    agent("fork_thread"),
    agent("wiki_ref_drift"),
    // ---- collection (effort-scoped observations) ----
    agent("ingest_coverage"),
    agent("ingest_analysis"),
    agent("record_test_run"),
    agent("get_open_effort"),
    agent("code_definition"),
    agent("code_hover"),
    agent("code_references"),
    agent("code_symbols"),
    agent("code_workspace_symbols"),
    agent("code_call_hierarchy"),
    agent("code_diagnostics"),
    agent("lsp_list_servers"),
    agent("lsp_install_server"),
    // ---- code analysis: generic per-language unit listing (tree-sitter) ----
    agent("list_code_units"),
    // ---- git: read tools mirrored to MCP (Child 2) ----
    both_named("git.status", "git_change_scopes", "git_status"),
    both("diff"),
    both("vcs_log"),
    both("vcs_blame"),
    both("read_at"),
    both("vcs_branches"),
    // ---- agent_todo: git reads/mutations still on Bash (deferred) ----
    todo("search_workspace_text"),
    // ---- snapshots / local history: reads + restore mirrored to MCP (Child 3) ----
    both("list_snapshots_for_stream"),
    both("list_snapshot_ops"),
    both("list_files_for_snapshot"),
    both("get_file_snapshot"),
    both("get_snapshot_stats"),
    agent("list_snapshot_change_entries"),
    agent("read_file_snapshot"),
    agent("read_file_at_snapshot"),
    both("read_event_content"),
    both("restore_file_snapshot"),
    // Endpoint diff for the diff view page (effort / local-history) — UI-only.
    // Per-file content at an endpoint, feeding the diff view's function
    // analysis (base + head). UI-only.
    // ---- agent_todo: composed dashboard DTOs / generated-filtered (deferred) ----
    todo("list_file_snapshots"),
    // ---- code quality: duplication findings mirrored to MCP (metrics scan
    //      retired in tsk229; signals moved to the metric substrate) ----
    agent("list_code_quality_findings"),
    // ---- comments + stream/thread lifecycle mirrored to MCP (Child 5) ----
    both("create_comment"),
    both("set_comment_intent"),
    both("rename_thread"),
    both("close_thread"),
    both("reopen_thread"),
    both("select_thread"),
    both("promote_thread"),
    both("switch_stream"),
    both("rename_stream"),
    // checkout stays on Bash — subprocess logic lives in the IPC command layer.
    // ---- ui-only: app / misc ----
    ui("log_ui"),
    // ---- ui-only: streams ----
    ui("create_worktree"),
    ui("adopt_worktree"),
    ui("archive_stream"),
    ui("get_primary_stream"),
    ui("get_current_stream"),
    ui("set_stream_prompt"),
    ui("reorder_streams"),
    // ---- ui-only: threads ----
    ui("create_thread"),
    // The thread picker's ACP agents (tsk335).
    ui("list_acp_agents"),
    // ACP sessions: the prompt box, permission cards and banners. Never
    // agent tools — an agent must not prompt an agent (tsk281).
    ui("acp_open_session"),
    ui("acp_prompt"),
    ui("acp_cancel"),
    ui("acp_respond_permission"),
    ui("acp_transcript"),
    ui("acp_dismiss_directive"),
    ui("acp_close_session"),
    ui("set_thread_prompt"),
    ui("set_agents"),
    ui("list_closed_threads"),
    ui("reorder_thread_queue"),
    ui("get_thread_state"),
    // ---- ui-only: tasks / backlog ----
    // ---- dashboards (tsk138) — reads + create/add-tile are agent-authorable
    // (both); the rest are pure-UI edits (tsk140). ----
    both("list_dashboards"),
    both("get_dashboard"),
    both("create_dashboard"),
    both("add_dashboard_item"),
    ui("rename_dashboard"),
    ui("delete_dashboard"),
    ui("update_dashboard_item"),
    ui("remove_dashboard_item"),
    ui("reorder_dashboard_items"),
    // ---- ui-only: comments (anchor management / destructive) ----
    ui("list_comments_for_target"),
    ui("set_comment_anchor"),
    ui("relink_comment"),
    ui("delete_comment"),
    // ---- ui-only: wiki (writes are the knowledge.* commands) ----
    // ---- ui-only: wiki freshness ----
    // ---- ui-only: page visits ----
    ui("record_page_visit"),
    ui("list_recent_page_visits"),
    ui("top_visited_pages"),
    ui("forget_page"),
    // ---- ui-only: usage ----
    ui("record_usage"),
    ui("list_recent_usage_rollup"),
    // ---- ui-only: code quality (UI-internal analysis helpers) ----
    // ---- ui-only: snapshots (UI presentation helpers) ----
    ui("list_wiki_slugs_for_snapshots"),
    ui("get_blob_storage_bytes"),
    // ---- ui-only: branches (remote/ref presentation) ----
    // ---- ui-only: git (presentation / worktree / remote helpers) ----
    ui("git_resolve_commit_ref_labels"),
    ui("git_list_recent_remote_branches"),
    ui("vcs_list_adoptable_workspaces"),
    ui("vcs_divergence"),
    ui("vcs_revisions_between"),
    ui("vcs_file_history"),
    // ---- ui-only: hooks / agent lifecycle ----
    ui("ingest_hook_event"),
    ui("list_agent_events"),
    ui("list_agent_statuses"),
    ui("list_open_agent_turns"),
    ui("get_agent_turn"),
    // ---- ui-only: config ----
    ui("get_config"),
    ui("set_agent_prompt_append"),
    ui("set_generated"),
    ui("set_agent_model"),
    ui("get_workspace_context"),
    // ---- ui-only: efforts ----
    ui("get_effort_files"),
    ui("get_effort"),
    ui("list_efforts_at_snapshots"),
    ui("list_efforts_overlapping_range"),
    ui("list_changed_paths_for_effort"),
    // ---- ui-only: git log (presentation) ----
    // ---- ui-only: workspace file I/O (agent uses Read/Write tools) ----
    ui("list_workspace_entries"),
    ui("list_workspace_files"),
    ui("read_workspace_file"),
    ui("files_at"),
    ui("vcs_head"),
    ui("vcs_status"),
    ui("vcs_revision"),
    ui("vcs_merge_base"),
    ui("write_workspace_file"),
    ui("create_workspace_file"),
    ui("create_workspace_directory"),
    ui("rename_workspace_path"),
    ui("delete_workspace_path"),
    // ---- ui-only: background tasks ----
    ui("list_background_tasks"),
    ui("get_background_task"),
    ui("start_background_task"),
    ui("complete_background_task"),
    ui("fail_background_task"),
    ui("update_background_task"),
    // ---- ui-only: webview ----
    ui("open_external_url"),
    ui("clipboard_read_text"),
    // ---- ui-only: lsp (shared sessions + installer) ----
    ui("install_lsp_package"),
    ui("list_installed_lsp_packages"),
    ui("lsp_request"),
    ui("lsp_notify"),
    ui("list_lsp_servers"),
    ui("restart_lsp_server"),
    ui("remove_lsp_package"),
    ui("respond_lsp_apply_edit"),
    // ---- ui-only: terminal ----
    // `forward_terminal_input` is UI-ONLY by design and must never reach
    // the MCP (agent) surface: it is the human keystroke/paste transport,
    // not an automation API. Keeping it `ui(...)` here is part of the
    // no-automation guard (see `.context/agent-model.md`).
    ui("open_terminal_session"),
    ui("forward_terminal_input"),
    ui("close_terminal_session"),
    ui("terminate_terminal_session"),
    ui("terminal_session_cwd"),
    // Read-only sessionId lookup (no spawn). UI/second-client only —
    // it feeds `forward_terminal_input`, which is itself UI-only by the
    // no-automation guard, so the agent has no use for it either.
    ui("lookup_terminal_session"),
    // ---- ui-only: menu ----
    ui("set_native_menu"),
    // ---- ui-only: launcher / multi-window ----
    ui("list_recent_projects"),
    ui("remove_recent_project"),
    ui("open_project"),
    ui("create_project"),
    ui("setup_project"),
    ui("abort_setup"),
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
            UiOnly => c.ipc.is_some() && c.mcp.is_none(),
            AgentOnly => c.ipc.is_none() && c.mcp.is_some(),
            AgentTodo => c.ipc.is_some() && c.mcp.is_none(),
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
