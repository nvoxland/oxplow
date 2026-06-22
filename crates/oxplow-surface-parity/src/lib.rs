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
    both("list_streams"),
    ui("list_backlog"),
    both("get_task"),
    both("create_task"),
    both("update_task"),
    both("upsert_task"),
    both("delete_task"),
    both("reorder_tasks"),
    both("add_thread_note"),
    both("list_thread_notes"),
    both("list_effort_observations"),
    // Unified metric substrate reads (tsk213) — exposed on both surfaces.
    both("list_metric_definitions"),
    both("list_metric_samples"),
    // Metric authoring/drill-in tools (tsk213, P3) — agent-only: the renderer
    // drives compute via config + the runner, not ad-hoc IPC.
    agent("run_metric"),
    agent("record_metric"),
    agent("list_metric_findings"),
    agent("get_metric_summary"),
    ui("list_nudges_for_effort"),
    ui("list_token_usage_for_effort"),
    ui("get_effort_token_totals"),
    ui("get_thread_token_totals"),
    ui("token_totals_overall"),
    ui("token_usage_by_agent"),
    ui("token_usage_by_model"),
    ui("token_usage_by_day"),
    both("list_wiki_pages"),
    both("add_followup"),
    both("list_followups"),
    both("remove_followup"),
    both("list_backlinks"),
    both("list_outbound"),
    both("search"),
    // ---- both (names diverge across surfaces) ----
    both_named("thread.list", "list_threads", "list_thread_work"),
    agent("list_tasks"),
    both_named(
        "wiki.search_titles",
        "search_wiki_titles",
        "search_wiki_pages",
    ),
    agent("search_wiki_page_bodies"),
    agent("get_wiki_page_metadata"),
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
    agent("link_tasks"),
    agent("transition_tasks"),
    agent("dispatch_task"),
    agent("get_thread_context"),
    agent("file_epic_with_children"),
    agent("delegate_query"),
    agent("record_query_finding"),
    agent("await_user"),
    agent("fork_thread"),
    agent("list_stale_wiki_pages"),
    agent("wiki_ref_drift"),
    agent("resync_wiki_page"),
    agent("record_wiki_page_update"),
    agent("find_wiki_pages_for_wiki_page"),
    // ---- collection (effort-scoped observations) ----
    agent("ingest_coverage"),
    agent("ingest_analysis"),
    agent("record_test_run"),
    agent("get_open_effort"),
    agent("lsp_definition"),
    agent("lsp_hover"),
    agent("lsp_references"),
    agent("lsp_diagnostics"),
    agent("lsp_list_servers"),
    agent("lsp_install_server"),
    // ---- git: read tools mirrored to MCP (Child 2) ----
    both_named("git.status", "get_change_scopes", "git_status"),
    both_named("git.diff", "get_branch_changes", "git_diff"),
    both_named("git.log", "get_git_log", "git_log"),
    both("git_blame"),
    both("read_file_at_ref"),
    both("list_branches"),
    // ---- agent_todo: git reads/mutations still on Bash (deferred) ----
    todo("get_commit_detail"),
    todo("get_commits_ahead_of"),
    todo("get_ahead_behind"),
    todo("list_stream_divergences"),
    todo("list_file_commits"),
    todo("search_workspace_text"),
    todo("list_local_branches"),
    todo("get_repo_conflict_state"),
    todo("restore_path"),
    todo("git_fetch"),
    todo("git_pull"),
    todo("git_pull_remote_into_current"),
    todo("git_push"),
    todo("git_push_current_to"),
    todo("git_merge_into"),
    todo("git_rebase_onto"),
    todo("git_cherry_pick"),
    todo("git_revert"),
    todo("git_commit_all"),
    todo("git_add_path"),
    // ---- snapshots / local history: reads + restore mirrored to MCP (Child 3) ----
    both("list_snapshots_for_stream"),
    both("list_files_for_snapshot"),
    both("get_snapshot"),
    both("get_snapshot_stats"),
    both("list_snapshot_change_entries"),
    both("read_snapshot_file_content"),
    both("restore_file_from_snapshot"),
    // ---- agent_todo: composed dashboard DTOs / generated-filtered (deferred) ----
    todo("list_snapshots"),
    todo("get_snapshot_pair_diff"),
    todo("get_snapshot_summary"),
    // ---- code quality: duplication findings mirrored to MCP (metrics scan
    //      retired in tsk229; signals moved to the metric substrate) ----
    both("list_code_quality_findings"),
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
    todo("checkout_stream_branch"),
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
    ui("set_thread_prompt"),
    ui("set_agents"),
    ui("list_closed_threads"),
    ui("reorder_thread_queue"),
    ui("get_thread_state"),
    ui("get_thread_work_state"),
    // ---- ui-only: tasks / backlog ----
    ui("move_task"),
    ui("get_backlog_state"),
    // ---- ui-only: notes ----
    ui("list_task_events"),
    // ---- ui-only: comments (anchor management / destructive) ----
    ui("list_comments_for_target"),
    ui("set_comment_anchor"),
    ui("relink_comment"),
    ui("delete_comment"),
    // ---- ui-only: wiki (bodies stay direct file writes) ----
    ui("upsert_wiki_page"),
    both("delete_wiki_page"),
    ui("read_wiki_page_body"),
    ui("write_wiki_page_body"),
    // ---- ui-only: wiki freshness ----
    ui("list_wiki_freshness"),
    ui("mark_wiki_ref_verified"),
    ui("mark_all_wiki_refs_verified"),
    // ---- ui-only: page visits ----
    ui("record_page_visit"),
    ui("list_recent_page_visits"),
    ui("top_visited_pages"),
    ui("forget_page"),
    ui("count_page_visits_by_day"),
    ui("list_recently_finished"),
    ui("clear_recently_finished"),
    // ---- ui-only: usage ----
    ui("record_usage"),
    ui("list_recent_usage_rollup"),
    // ---- ui-only: code quality (UI-internal analysis helpers) ----
    ui("run_duplication_scan_at"),
    ui("find_latest_code_quality_scan"),
    ui("analyze_functions_at_refs"),
    ui("analyze_co_change_surprise"),
    // ---- ui-only: snapshots (UI presentation helpers) ----
    ui("list_file_snapshots_for_stream"),
    ui("list_wiki_slugs_for_snapshots"),
    ui("get_blob_storage_bytes"),
    // ---- ui-only: branches (remote/ref presentation) ----
    ui("get_default_branch"),
    ui("rename_branch"),
    ui("delete_branch"),
    // ---- ui-only: git (presentation / worktree / remote helpers) ----
    ui("append_to_gitignore"),
    ui("list_all_refs"),
    ui("resolve_commit_ref_labels"),
    ui("list_recent_remote_branches"),
    ui("list_adoptable_worktrees"),
    ui("local_blame"),
    // ---- ui-only: hooks / agent lifecycle ----
    ui("ingest_hook_event"),
    ui("list_hook_events"),
    ui("list_agent_statuses"),
    ui("list_open_agent_turns"),
    // ---- ui-only: config ----
    ui("get_config"),
    ui("set_agent_prompt_append"),
    ui("set_snapshot_retention_days"),
    ui("set_snapshot_max_file_bytes"),
    ui("set_generated"),
    ui("set_agent_model"),
    ui("get_workspace_context"),
    // ---- ui-only: efforts ----
    ui("list_task_efforts"),
    ui("get_effort_files"),
    ui("list_efforts_at_snapshots"),
    ui("list_changed_paths_for_effort"),
    // ---- ui-only: git log (presentation) ----
    ui("get_workspace_status_summary"),
    // ---- ui-only: workspace file I/O (agent uses Read/Write tools) ----
    ui("list_workspace_entries"),
    ui("list_workspace_files"),
    ui("read_workspace_file"),
    ui("read_file"),
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
    ui("get_launch_mode"),
    ui("list_recent_projects"),
    ui("remove_recent_project"),
    ui("open_project"),
    ui("project_needs_setup"),
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
}
