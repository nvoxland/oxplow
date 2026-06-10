//! Tauri command + event adapter.
//!
//! Each `#[tauri::command]` is a thin wrapper around an `oxplow-app`
//! service method. Errors convert at this boundary into the
//! frontend-facing `IpcError`. `tauri-specta` exports the typed JS
//! bindings consumed by `apps/desktop/src/tauri-bridge/`.

pub mod commands;
pub mod error;
pub mod state;

pub use error::IpcError;
pub use state::{AppState, LaunchInfo, PluginRuntime, PluginRuntimeState, RecentProjectsState};

use tauri_specta::{collect_commands, Builder};

pub use oxplow_app::OxplowEvent;

/// Stable event channel name used by the renderer's `listen` calls.
/// Payload is `OxplowEvent` JSON.
pub const OXPLOW_EVENT_CHANNEL: &str = "oxplow:event";

/// Build the tauri-specta `Builder` registering every oxplow command.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        // app
        commands::app::app_version,
        commands::app::ping,
        commands::app::log_ui,
        // streams
        commands::streams::list_streams,
        commands::streams::ensure_primary,
        commands::streams::create_worktree,
        commands::streams::adopt_worktree,
        commands::streams::delete_stream,
        commands::streams::archive_stream,
        commands::streams::get_primary_stream,
        commands::streams::get_current_stream,
        commands::streams::switch_stream,
        commands::streams::rename_stream,
        commands::streams::set_stream_prompt,
        commands::streams::checkout_stream_branch,
        commands::streams::reorder_streams,
        // threads
        commands::threads::list_threads,
        commands::threads::get_thread,
        commands::threads::upsert_thread,
        commands::threads::delete_thread,
        commands::threads::create_thread,
        commands::threads::rename_thread,
        commands::threads::set_thread_prompt,
        commands::threads::promote_thread,
        commands::threads::close_thread,
        commands::threads::reopen_thread,
        commands::threads::list_closed_threads,
        commands::threads::reorder_thread_queue,
        commands::threads::get_selected_thread,
        commands::threads::select_thread,
        commands::threads::get_thread_state,
        commands::threads::get_thread_work_state,
        // tasks
        commands::tasks::list_tasks_for_thread,
        commands::tasks::get_task,
        commands::tasks::upsert_task,
        commands::tasks::delete_task,
        commands::tasks::create_task,
        commands::tasks::update_task,
        commands::tasks::reorder_tasks,
        commands::tasks::move_task,
        commands::tasks::get_task_summaries,
        // backlog
        commands::backlog::list_backlog,
        commands::backlog::get_backlog_state,
        // search
        commands::search::search,
        // notes (task / thread)
        commands::notes::add_thread_note,
        commands::notes::list_thread_notes,
        commands::notes::delete_work_note,
        commands::notes::list_task_events,
        // comments
        commands::comments::create_comment,
        commands::comments::add_comment_message,
        commands::comments::list_comments_for_target,
        commands::comments::list_comments_for_stream,
        commands::comments::set_comment_intent,
        commands::comments::set_comment_status,
        commands::comments::set_comment_anchor,
        commands::comments::relink_comment,
        commands::comments::delete_comment,
        // wiki
        commands::wiki::list_wiki_pages,
        commands::wiki::get_wiki_page,
        commands::wiki::upsert_wiki_page,
        commands::wiki::delete_wiki_page,
        commands::wiki::search_wiki_titles,
        commands::wiki::search_wiki_bodies,
        commands::wiki::read_wiki_page_body,
        commands::wiki::write_wiki_page_body,
        // page refs (unified backlinks/outbound)
        commands::page_refs::list_backlinks,
        commands::page_refs::list_outbound,
        // wiki freshness (V20 file-ref version tracking)
        commands::wiki_freshness::list_wiki_freshness,
        commands::wiki_freshness::mark_wiki_ref_verified,
        commands::wiki_freshness::mark_all_wiki_refs_verified,
        // page visit
        commands::page_visit::record_page_visit,
        commands::page_visit::list_recent_page_visits,
        commands::page_visit::top_visited_pages,
        commands::page_visit::forget_page,
        commands::page_visit::count_page_visits_by_day,
        commands::page_visit::list_frequent_usage,
        commands::page_visit::list_currently_open_usage,
        commands::page_visit::list_recently_finished,
        commands::page_visit::clear_recently_finished,
        // usage
        commands::usage::record_usage,
        commands::usage::list_recent_usage,
        commands::usage::list_recent_usage_rollup,
        // code quality
        commands::code_quality::list_code_quality_scans,
        commands::code_quality::list_code_quality_findings,
        commands::code_quality::run_code_quality_scan,
        commands::code_quality::run_duplication_scan_at,
        commands::code_quality::find_latest_code_quality_scan,
        commands::code_quality::analyze_functions_at_refs,
        commands::code_quality::analyze_co_change_surprise,
        // snapshots
        commands::snapshot::list_snapshots,
        commands::snapshot::list_file_snapshots_for_stream,
        commands::snapshot::list_snapshots_for_stream,
        commands::snapshot::list_files_for_snapshot,
        commands::snapshot::list_wiki_slugs_for_snapshots,
        commands::snapshot::get_snapshot,
        commands::snapshot::get_snapshot_pair_diff,
        commands::snapshot::get_snapshot_summary,
        commands::snapshot::get_snapshot_stats,
        commands::snapshot::list_snapshot_change_entries,
        commands::snapshot::read_snapshot_file_content,
        commands::snapshot::get_blob_storage_bytes,
        commands::snapshot::restore_file_from_snapshot,
        // branch
        commands::branch::list_branches,
        commands::branch::get_default_branch,
        commands::branch::rename_branch,
        commands::branch::delete_branch,
        commands::branch::list_local_branches,
        // git
        commands::git::get_repo_conflict_state,
        commands::git::get_ahead_behind,
        commands::git::append_to_gitignore,
        commands::git::restore_path,
        commands::git::git_fetch,
        commands::git::git_pull,
        commands::git::git_pull_remote_into_current,
        commands::git::git_push,
        commands::git::git_push_current_to,
        commands::git::git_merge_into,
        commands::git::git_rebase_onto,
        commands::git::git_commit_all,
        commands::git::git_add_path,
        commands::git::list_all_refs,
        commands::git::resolve_commit_ref_labels,
        commands::git::list_recent_remote_branches,
        commands::git::list_file_commits,
        commands::git::read_file_at_ref,
        commands::git::search_workspace_text,
        commands::git::list_existing_worktrees,
        commands::git::list_adoptable_worktrees,
        commands::git::git_blame,
        commands::git::local_blame,
        commands::git::get_branch_changes,
        commands::git::get_change_scopes,
        // hooks / agent lifecycle
        commands::hooks::ingest_hook_event,
        commands::hooks::list_hook_events,
        commands::hooks::list_hook_events_by_kind,
        commands::hooks::list_agent_statuses,
        commands::hooks::list_open_agent_turns,
        commands::hooks::list_recent_agent_turns,
        // config
        commands::config::get_config,
        commands::config::set_agent_prompt_append,
        commands::config::set_agents,
        commands::config::set_snapshot_retention_days,
        commands::config::set_snapshot_max_file_bytes,
        commands::config::set_generated,
        commands::config::get_workspace_context,
        // agent panes
        commands::agent_panes::ensure_agent_pane,
        commands::agent_panes::teardown_agent_panes,
        // efforts
        commands::effort::list_task_efforts,
        commands::effort::get_effort_files,
        commands::effort::list_efforts_at_snapshots,
        commands::effort::list_changed_paths_for_effort,
        commands::effort::list_effort_observations,
        // log
        commands::log::get_git_log,
        commands::log::get_commit_detail,
        commands::log::get_commits_ahead_of,
        // workspace
        commands::workspace::list_workspace_entries,
        commands::workspace::list_workspace_files,
        commands::workspace::read_workspace_file,
        commands::workspace::read_file,
        commands::workspace::write_workspace_file,
        commands::workspace::create_workspace_file,
        commands::workspace::create_workspace_directory,
        commands::workspace::rename_workspace_path,
        commands::workspace::delete_workspace_path,
        commands::workspace::get_workspace_status_summary,
        // background tasks
        commands::background::list_background_tasks,
        commands::background::get_background_task,
        commands::background::start_background_task,
        commands::background::complete_background_task,
        commands::background::fail_background_task,
        commands::background::update_background_task,
        // followups
        commands::followup::list_followups,
        commands::followup::add_followup,
        commands::followup::remove_followup,
        commands::followup::clear_followups_for_thread,
        // webview
        commands::webview::open_external_url,
        commands::webview::clipboard_read_text,
        // lsp
        commands::lsp::open_lsp_client,
        commands::lsp::send_lsp_message,
        commands::lsp::close_lsp_client,
        commands::lsp::install_lsp_package,
        commands::lsp::list_installed_lsp_packages,
        // terminal
        commands::terminal::open_terminal_session,
        commands::terminal::send_terminal_message,
        commands::terminal::close_terminal_session,
        commands::terminal::terminal_session_cwd,
        commands::terminal::terminate_terminal_session,
        // menu
        commands::menu::set_native_menu,
        // launcher / multi-window
        commands::launch::get_launch_mode,
        commands::launch::list_recent_projects,
        commands::launch::remove_recent_project,
        commands::launch::open_project,
        commands::launch::project_needs_setup,
        commands::launch::setup_project,
        commands::launch::abort_setup,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the builder constructs without panicking.
    #[test]
    fn builder_constructs() {
        let _b = specta_builder();
    }

    /// Regenerate the TS bindings file the frontend imports.
    /// CI fails if `git diff` is non-empty after `cargo test`.
    #[test]
    fn export_ts_bindings() {
        let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(v) => v,
            Err(_) => return,
        };
        let workspace_root = std::path::Path::new(&manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root");
        let target = workspace_root.join("apps/desktop/src/tauri-bridge/generated/bindings.ts");
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create bridge dir");
        }
        let builder = specta_builder();
        builder
            .export(specta_typescript::Typescript::default(), &target)
            .expect("export bindings");
        // Route every generated invoke through the transport switch so
        // remote mode (fetch to the daemon) works without touching the
        // ~130 call sites. tauri-specta has no hook for the import
        // path, so rewrite it post-export; local mode is unchanged
        // because ../transport delegates to @tauri-apps/api/core.
        let generated = std::fs::read_to_string(&target).expect("read bindings");
        let routed = generated.replace(
            "import { invoke as __TAURI_INVOKE } from \"@tauri-apps/api/core\";",
            "import { invoke as __TAURI_INVOKE } from \"../transport\";",
        );
        assert!(
            routed.contains("from \"../transport\""),
            "expected to rewrite the bindings' invoke import; did tauri-specta change its header?"
        );
        std::fs::write(&target, routed).expect("write routed bindings");
        let metadata = std::fs::metadata(&target).expect("bindings written");
        assert!(metadata.len() > 0, "bindings file should not be empty");
    }
}
