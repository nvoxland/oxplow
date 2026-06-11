//! Transport-neutral command dispatch.
//!
//! Holds the command "core" functions ([`commands`]) and a name-keyed
//! [`dispatch`] registry that turns `(name, JSON args, &Services)` into a
//! `Result<JSON, IpcError>`. Both the local Tauri command wrappers
//! (`oxplow-tauri-ipc`) and the headless HTTP daemon (`oxplow-daemon`)
//! route through the same cores, so the command set has a single source
//! of truth and the daemon needs no `tauri` dependency.
//!
//! ## Wire shape
//!
//! Arguments arrive as the exact object the renderer already sends to
//! Tauri's `invoke` — camelCase keys (`{ threadId, newWindow }`). The
//! [`rpc_dispatch!`] macro builds a private per-command `Args` struct
//! with `#[serde(rename_all = "camelCase")]` so that mapping is handled
//! by serde rather than hand-written key lookups. A no-arg command
//! accepts `null`/absent body or `{}` interchangeably.
//!
//! The registry below is seeded with a representative slice (no-arg,
//! service-only, and single-arg commands) and is extended one module at
//! a time as the remaining commands are migrated.

pub mod commands;
pub mod error;

pub use error::IpcError;

use std::sync::Arc;

use oxplow_app::Services;

/// Control-plane coordinates an agent spawn needs: where the plugin's
/// HTTP hooks POST to, the MCP endpoint, and the bearer token both
/// use. The Tauri shell materializes this from its in-process control
/// plane; the daemon from its own. Lives here (not oxplow-tauri-ipc)
/// so the headless daemon can construct it without tauri deps.
#[derive(Clone, Debug)]
pub struct PluginRuntime {
    pub hook_base_url: String,
    pub mcp_endpoint_url: String,
    pub hook_token: String,
}

/// Everything a dispatched command may need. Most cores only touch
/// `services` (and the registry hands them `&Services` directly);
/// agent-spawning commands also need `plugin_runtime`. `None` means
/// the host can't support agent spawn — the affected commands return
/// INVALID instead of panicking.
#[derive(Clone)]
pub struct RpcContext {
    pub services: Arc<Services>,
    pub plugin_runtime: Option<PluginRuntime>,
}

/// Cores and helpers overwhelmingly want `&Services`; deref so a
/// context behaves like one wherever that's all that's needed.
impl std::ops::Deref for RpcContext {
    type Target = Services;
    fn deref(&self) -> &Services {
        &self.services
    }
}

/// Build the [`dispatch`] function from two sections of
/// `"wire_name" => core_fn { field: Type, ... }` entries:
///
/// - `svc { ... }` — the common case; the core's first parameter is
///   `&Services`.
/// - `ctx { ... }` — commands that need more than the services (today:
///   agent spawn, which reads `plugin_runtime`); the core's first
///   parameter is `&RpcContext`.
///
/// Each entry generates a match arm that deserializes the args object
/// into a private camelCase struct, destructures it, and calls the
/// core. An empty field list means no args beyond the context.
#[macro_export]
macro_rules! rpc_dispatch {
    (
        ctx { $( $cname:literal => $ccore:path { $( $cfield:ident : $cfty:ty ),* $(,)? } ),* $(,)? }
        svc { $( $name:literal => $core:path { $( $field:ident : $fty:ty ),* $(,)? } ),* $(,)? }
    ) => {
        /// Route a command by name to its core, deserializing `args`
        /// (the renderer's camelCase invoke payload) and re-serializing
        /// the result. An unknown name yields `NOT_FOUND`.
        pub async fn dispatch(
            name: &str,
            args: serde_json::Value,
            ctx: &$crate::RpcContext,
        ) -> Result<serde_json::Value, $crate::IpcError> {
            // The renderer omits the body for no-arg commands; normalize
            // `null`/absent to an empty object so the (possibly empty)
            // Args struct still deserializes.
            let args = if args.is_null() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                args
            };
            match name {
                $(
                    $cname => {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args {
                            $( $cfield : $cfty ),*
                        }
                        let Args { $( $cfield ),* } = serde_json::from_value(args).map_err(|e| {
                            $crate::IpcError::invalid(format!("bad args for {name}: {e}"))
                        })?;
                        let out = $ccore(ctx $(, $cfield )*).await?;
                        serde_json::to_value(out).map_err(|e| {
                            $crate::IpcError::internal(format!("serialize result of {name}: {e}"))
                        })
                    }
                )*
                $(
                    $name => {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args {
                            $( $field : $fty ),*
                        }
                        let Args { $( $field ),* } = serde_json::from_value(args).map_err(|e| {
                            $crate::IpcError::invalid(format!("bad args for {name}: {e}"))
                        })?;
                        let out = $core(ctx.services.as_ref() $(, $field )*).await?;
                        serde_json::to_value(out).map_err(|e| {
                            $crate::IpcError::internal(format!("serialize result of {name}: {e}"))
                        })
                    }
                )*
                _ => Err($crate::IpcError::not_found()),
            }
        }
    };
}

rpc_dispatch! {
    ctx {
        // terminal — the agent-spawn path needs plugin_runtime
        "open_terminal_session" => commands::terminal::open_terminal_session { pane_target: String, cols: u16, rows: u16, transport_mode: String },
    }
    svc {
    // app
    "ping" => commands::app::ping {},
    "app_version" => commands::app::app_version {},
    "log_ui" => commands::app::log_ui { entry: crate::commands::app::UiLogEntry },
    // streams
    "list_streams" => commands::streams::list_streams {},
    "ensure_primary" => commands::streams::ensure_primary {},
    "create_worktree" => commands::streams::create_worktree { req: crate::commands::streams::CreateWorktreeRequest },
    "adopt_worktree" => commands::streams::adopt_worktree { req: crate::commands::streams::AdoptWorktreeRequest },
    "delete_stream" => commands::streams::delete_stream { id: oxplow_domain::StreamId },
    "archive_stream" => commands::streams::archive_stream { id: oxplow_domain::StreamId, delete_worktree: bool },
    "get_primary_stream" => commands::streams::get_primary_stream {},
    "get_current_stream" => commands::streams::get_current_stream {},
    "switch_stream" => commands::streams::switch_stream { id: Option<oxplow_domain::StreamId> },
    "rename_stream" => commands::streams::rename_stream { req: crate::commands::streams::RenameStreamRequest },
    "set_stream_prompt" => commands::streams::set_stream_prompt { req: crate::commands::streams::SetStreamPromptRequest },
    "reorder_streams" => commands::streams::reorder_streams { order: Vec<oxplow_domain::StreamId> },
    "checkout_stream_branch" => commands::streams::checkout_stream_branch { id: oxplow_domain::StreamId, branch: String },
    // branch
    "list_branches" => commands::branch::list_branches {},
    "get_default_branch" => commands::branch::get_default_branch {},
    "rename_branch" => commands::branch::rename_branch { from: String, to: String },
    "delete_branch" => commands::branch::delete_branch { branch: String, force: bool },
    "list_local_branches" => commands::branch::list_local_branches {},
    // agent_panes
    "ensure_agent_pane" => commands::agent_panes::ensure_agent_pane { req: crate::commands::agent_panes::EnsureAgentPaneRequest },
    "teardown_agent_panes" => commands::agent_panes::teardown_agent_panes { stream_id: oxplow_domain::StreamId },
    // threads
    "list_threads" => commands::threads::list_threads { stream_id: oxplow_domain::StreamId },
    "get_thread" => commands::threads::get_thread { thread_id: oxplow_domain::ThreadId },
    "upsert_thread" => commands::threads::upsert_thread { thread: oxplow_domain::Thread },
    "delete_thread" => commands::threads::delete_thread { thread_id: oxplow_domain::ThreadId },
    "create_thread" => commands::threads::create_thread { req: crate::commands::threads::CreateThreadRequest },
    "rename_thread" => commands::threads::rename_thread { req: crate::commands::threads::RenameThreadRequest },
    "set_thread_prompt" => commands::threads::set_thread_prompt { req: crate::commands::threads::SetThreadPromptRequest },
    "promote_thread" => commands::threads::promote_thread { id: oxplow_domain::ThreadId },
    "close_thread" => commands::threads::close_thread { id: oxplow_domain::ThreadId },
    "reopen_thread" => commands::threads::reopen_thread { id: oxplow_domain::ThreadId },
    "list_closed_threads" => commands::threads::list_closed_threads { stream_id: oxplow_domain::StreamId },
    "reorder_thread_queue" => commands::threads::reorder_thread_queue { req: crate::commands::threads::ReorderThreadQueueRequest },
    "get_selected_thread" => commands::threads::get_selected_thread { stream_id: oxplow_domain::StreamId },
    "get_thread_state" => commands::threads::get_thread_state { stream_id: oxplow_domain::StreamId },
    "get_thread_work_state" => commands::threads::get_thread_work_state { thread_id: oxplow_domain::ThreadId },
    "select_thread" => commands::threads::select_thread { req: crate::commands::threads::SelectThreadRequest },
    // backlog
    "list_backlog" => commands::backlog::list_backlog {},
    "get_backlog_state" => commands::backlog::get_backlog_state {},
    // notes
    "add_thread_note" => commands::notes::add_thread_note { thread_id: oxplow_domain::ThreadId, body: String, author: String },
    "list_thread_notes" => commands::notes::list_thread_notes { thread_id: oxplow_domain::ThreadId },
    "delete_work_note" => commands::notes::delete_work_note { id: oxplow_domain::NoteId },
    "list_task_events" => commands::notes::list_task_events { item_id: Option<oxplow_domain::TaskId>, thread_id: Option<oxplow_domain::ThreadId> },
    // tasks
    "list_tasks_for_thread" => commands::tasks::list_tasks_for_thread { thread_id: oxplow_domain::ThreadId },
    "get_task" => commands::tasks::get_task { id: oxplow_domain::TaskId },
    "upsert_task" => commands::tasks::upsert_task { item: oxplow_domain::Task },
    "delete_task" => commands::tasks::delete_task { id: oxplow_domain::TaskId },
    "create_task" => commands::tasks::create_task { req: crate::commands::tasks::CreateTaskRequest },
    "update_task" => commands::tasks::update_task { req: crate::commands::tasks::UpdateTaskRequest },
    "reorder_tasks" => commands::tasks::reorder_tasks { req: crate::commands::tasks::ReorderTasksRequest },
    "get_task_summaries" => commands::tasks::get_task_summaries { thread_id: Option<oxplow_domain::ThreadId> },
    "move_task" => commands::tasks::move_task { req: crate::commands::tasks::MoveTaskRequest },
    // effort
    "list_task_efforts" => commands::effort::list_task_efforts { item_id: oxplow_domain::TaskId },
    "get_effort_files" => commands::effort::get_effort_files { effort_id: oxplow_domain::EffortId },
    "list_efforts_at_snapshots" => commands::effort::list_efforts_at_snapshots { snapshot_ids: Vec<i64> },
    "list_changed_paths_for_effort" => commands::effort::list_changed_paths_for_effort { effort_id: oxplow_domain::EffortId },
    "list_effort_observations" => commands::effort::list_effort_observations { effort_id: oxplow_domain::EffortId, kind: Option<String> },
    // followup
    "list_followups" => commands::followup::list_followups { thread_id: oxplow_domain::ThreadId },
    "add_followup" => commands::followup::add_followup { thread_id: oxplow_domain::ThreadId, body: String },
    "remove_followup" => commands::followup::remove_followup { id: String },
    "clear_followups_for_thread" => commands::followup::clear_followups_for_thread { thread_id: oxplow_domain::ThreadId },
    // hooks
    "ingest_hook_event" => commands::hooks::ingest_hook_event { envelope: oxplow_app::HookEnvelope },
    "list_hook_events" => commands::hooks::list_hook_events { thread_id: Option<oxplow_domain::ThreadId>, limit: Option<usize> },
    "list_hook_events_by_kind" => commands::hooks::list_hook_events_by_kind { kind: oxplow_domain::HookKind, limit: Option<usize> },
    "list_agent_statuses" => commands::hooks::list_agent_statuses {},
    "list_open_agent_turns" => commands::hooks::list_open_agent_turns { thread_id: oxplow_domain::ThreadId },
    "list_recent_agent_turns" => commands::hooks::list_recent_agent_turns { thread_id: oxplow_domain::ThreadId, limit: Option<usize> },
    // wiki
    "list_wiki_pages" => commands::wiki::list_wiki_pages {},
    "get_wiki_page" => commands::wiki::get_wiki_page { slug: String },
    "upsert_wiki_page" => commands::wiki::upsert_wiki_page { note: oxplow_db::WikiPage },
    "delete_wiki_page" => commands::wiki::delete_wiki_page { slug: String },
    "search_wiki_titles" => commands::wiki::search_wiki_titles { query: String, limit: u32 },
    "search_wiki_bodies" => commands::wiki::search_wiki_bodies { query: String, limit: u32 },
    "read_wiki_page_body" => commands::wiki::read_wiki_page_body { slug: String },
    "write_wiki_page_body" => commands::wiki::write_wiki_page_body { slug: String, body: String },
    // page_refs
    "list_backlinks" => commands::page_refs::list_backlinks { target_kind: String, target_id: String, limit: Option<i64> },
    "list_outbound" => commands::page_refs::list_outbound { source_kind: String, source_id: String, limit: Option<i64> },
    // wiki_freshness
    "list_wiki_freshness" => commands::wiki_freshness::list_wiki_freshness { slug: String },
    "mark_wiki_ref_verified" => commands::wiki_freshness::mark_wiki_ref_verified { slug: String, path: String },
    "mark_all_wiki_refs_verified" => commands::wiki_freshness::mark_all_wiki_refs_verified { slug: String },
    // search
    "search" => commands::search::search { query: String, stream_id: Option<String>, kinds: Option<Vec<String>>, limit: Option<u32> },
    // comments
    "create_comment" => commands::comments::create_comment { req: crate::commands::comments::CreateCommentRequest },
    "add_comment_message" => commands::comments::add_comment_message { comment_id: oxplow_domain::CommentId, author: String, body: String },
    "list_comments_for_target" => commands::comments::list_comments_for_target { target_kind: String, target_id: String },
    "list_comments_for_stream" => commands::comments::list_comments_for_stream { stream_id: oxplow_domain::StreamId },
    "set_comment_intent" => commands::comments::set_comment_intent { comment_id: oxplow_domain::CommentId, intent: oxplow_domain::CommentIntent },
    "set_comment_status" => commands::comments::set_comment_status { comment_id: oxplow_domain::CommentId, status: oxplow_domain::CommentStatus },
    "set_comment_anchor" => commands::comments::set_comment_anchor { comment_id: oxplow_domain::CommentId, selectors_json: String, orphaned: bool },
    "relink_comment" => commands::comments::relink_comment { comment_id: oxplow_domain::CommentId, quote: String, selectors_json: String },
    "delete_comment" => commands::comments::delete_comment { comment_id: oxplow_domain::CommentId },
    // page_visit
    "record_page_visit" => commands::page_visit::record_page_visit { page_kind: String, page_id: String, label: Option<String>, duration_ms: Option<i64>, thread_id: Option<String> },
    "list_recent_page_visits" => commands::page_visit::list_recent_page_visits { limit: u32, thread_id: Option<String> },
    "top_visited_pages" => commands::page_visit::top_visited_pages { limit: u32, thread_id: Option<String> },
    "forget_page" => commands::page_visit::forget_page { page_kind: String, page_id: String },
    "list_frequent_usage" => commands::page_visit::list_frequent_usage { limit: u32 },
    "list_currently_open_usage" => commands::page_visit::list_currently_open_usage { limit: u32 },
    "list_recently_finished" => commands::page_visit::list_recently_finished { thread_id: Option<String>, limit: u32 },
    "clear_recently_finished" => commands::page_visit::clear_recently_finished { thread_id: Option<String> },
    "count_page_visits_by_day" => commands::page_visit::count_page_visits_by_day { days: u32 },
    // usage
    "record_usage" => commands::usage::record_usage { kind: String, payload_json: String },
    "list_recent_usage" => commands::usage::list_recent_usage { limit: u32 },
    "list_recent_usage_rollup" => commands::usage::list_recent_usage_rollup { kind: String, stream_id: Option<String>, limit: u32 },
    // git
    "get_repo_conflict_state" => commands::git::get_repo_conflict_state { stream_id: Option<String> },
    "get_ahead_behind" => commands::git::get_ahead_behind { stream_id: Option<String>, base: String, head: String },
    "append_to_gitignore" => commands::git::append_to_gitignore { stream_id: Option<String>, entry: String },
    "restore_path" => commands::git::restore_path { stream_id: Option<String>, path: String },
    "git_fetch" => commands::git::git_fetch { stream_id: Option<String>, remote: Option<String> },
    "git_pull" => commands::git::git_pull { stream_id: Option<String> },
    "git_pull_remote_into_current" => commands::git::git_pull_remote_into_current { stream_id: Option<String>, remote: String, branch: String },
    "git_push" => commands::git::git_push { stream_id: Option<String> },
    "git_push_current_to" => commands::git::git_push_current_to { stream_id: Option<String>, remote: String, branch: String },
    "git_merge_into" => commands::git::git_merge_into { stream_id: Option<String>, source: String },
    "git_rebase_onto" => commands::git::git_rebase_onto { stream_id: Option<String>, onto: String },
    "git_commit_all" => commands::git::git_commit_all { stream_id: Option<String>, message: String },
    "git_add_path" => commands::git::git_add_path { stream_id: Option<String>, path: String },
    "list_all_refs" => commands::git::list_all_refs {},
    "resolve_commit_ref_labels" => commands::git::resolve_commit_ref_labels { shas: Vec<String> },
    "list_recent_remote_branches" => commands::git::list_recent_remote_branches { limit: Option<usize> },
    "list_file_commits" => commands::git::list_file_commits { stream_id: Option<String>, path: String, limit: Option<usize> },
    "git_blame" => commands::git::git_blame { stream_id: Option<String>, path: String },
    "local_blame" => commands::git::local_blame { stream_id: Option<String>, path: String, disk_text: String },
    "get_change_scopes" => commands::git::get_change_scopes { stream_id: Option<String> },
    "get_branch_changes" => commands::git::get_branch_changes { stream_id: Option<String>, base_ref: String },
    "list_existing_worktrees" => commands::git::list_existing_worktrees {},
    "list_adoptable_worktrees" => commands::git::list_adoptable_worktrees {},
    "search_workspace_text" => commands::git::search_workspace_text { stream_id: Option<String>, query: String, limit: Option<usize> },
    "read_file_at_ref" => commands::git::read_file_at_ref { r#ref: String, path: String },
    // workspace
    "read_file" => commands::workspace::read_file { stream_id: Option<String>, relative_path: String, version: oxplow_tree_source::TreeVersion },
    "list_workspace_entries" => commands::workspace::list_workspace_entries { stream_id: Option<String>, relative_path: String },
    "list_workspace_files" => commands::workspace::list_workspace_files { stream_id: Option<String> },
    "read_workspace_file" => commands::workspace::read_workspace_file { stream_id: Option<String>, relative_path: String },
    "write_workspace_file" => commands::workspace::write_workspace_file { stream_id: Option<String>, relative_path: String, content: String },
    "create_workspace_file" => commands::workspace::create_workspace_file { stream_id: Option<String>, relative_path: String, content: String },
    "create_workspace_directory" => commands::workspace::create_workspace_directory { stream_id: Option<String>, relative_path: String },
    "rename_workspace_path" => commands::workspace::rename_workspace_path { stream_id: Option<String>, from_path: String, to_path: String },
    "delete_workspace_path" => commands::workspace::delete_workspace_path { stream_id: Option<String>, relative_path: String },
    "get_workspace_status_summary" => commands::workspace::get_workspace_status_summary { stream_id: Option<String> },
    // code_quality
    "list_code_quality_scans" => commands::code_quality::list_code_quality_scans { limit: u32 },
    "list_code_quality_findings" => commands::code_quality::list_code_quality_findings { scan_id: i64 },
    "run_code_quality_scan" => commands::code_quality::run_code_quality_scan { tool: String, scope: String, files: Option<Vec<String>> },
    "run_duplication_scan_at" => commands::code_quality::run_duplication_scan_at { tree_version: oxplow_tree_source::TreeVersion, file_filter: crate::commands::code_quality::FileFilterSpec, scope: String },
    "find_latest_code_quality_scan" => commands::code_quality::find_latest_code_quality_scan { tool: String, tree_version: oxplow_tree_source::TreeVersion, file_filter: crate::commands::code_quality::FileFilterSpec },
    "analyze_co_change_surprise" => commands::code_quality::analyze_co_change_surprise { file_paths: Vec<String> },
    "analyze_functions_at_refs" => commands::code_quality::analyze_functions_at_refs { files: Vec<crate::commands::code_quality::AnalyzeFileSpec> },
    // config
    "get_config" => commands::config::get_config {},
    "set_agent_prompt_append" => commands::config::set_agent_prompt_append { text: String },
    "set_agents" => commands::config::set_agents { agents: Vec<oxplow_config::AgentKind> },
    "set_snapshot_retention_days" => commands::config::set_snapshot_retention_days { days: u32 },
    "set_snapshot_max_file_bytes" => commands::config::set_snapshot_max_file_bytes { bytes: u64 },
    "set_generated" => commands::config::set_generated { entries: Vec<String> },
    "get_workspace_context" => commands::config::get_workspace_context {},
    // lsp
    "install_lsp_package" => commands::lsp::install_lsp_package { package_name: String },
    "list_installed_lsp_packages" => commands::lsp::list_installed_lsp_packages {},
    "lsp_request" => commands::lsp::lsp_request { stream_id: String, language_id: String, method: String, params_json: String },
    "lsp_notify" => commands::lsp::lsp_notify { stream_id: String, language_id: String, method: String, params_json: String },
    "list_lsp_servers" => commands::lsp::list_lsp_servers {},
    "restart_lsp_server" => commands::lsp::restart_lsp_server { stream_id: String, language_id: String },
    "remove_lsp_package" => commands::lsp::remove_lsp_package { package_name: String },
    // terminal (open_terminal_session stays Tauri-only: PluginRuntimeState)
    "send_terminal_message" => commands::terminal::send_terminal_message { session_id: String, message: String },
    "close_terminal_session" => commands::terminal::close_terminal_session { session_id: String },
    "terminal_session_cwd" => commands::terminal::terminal_session_cwd { session_id: String },
    "terminate_terminal_session" => commands::terminal::terminate_terminal_session { session_id: String },
    // snapshot
    "list_snapshots" => commands::snapshot::list_snapshots { path: String },
    "list_file_snapshots_for_stream" => commands::snapshot::list_file_snapshots_for_stream { stream_id: oxplow_domain::StreamId, limit: Option<usize> },
    "list_snapshots_for_stream" => commands::snapshot::list_snapshots_for_stream { stream_id: oxplow_domain::StreamId, limit: Option<usize> },
    "get_snapshot_stats" => commands::snapshot::get_snapshot_stats { snapshot_id: i64 },
    "list_snapshot_change_entries" => commands::snapshot::list_snapshot_change_entries { snapshot_id: i64 },
    "read_snapshot_file_content" => commands::snapshot::read_snapshot_file_content { file_snapshot_id: i64 },
    "get_blob_storage_bytes" => commands::snapshot::get_blob_storage_bytes {},
    "list_wiki_slugs_for_snapshots" => commands::snapshot::list_wiki_slugs_for_snapshots { snapshot_ids: Vec<i64> },
    "list_files_for_snapshot" => commands::snapshot::list_files_for_snapshot { snapshot_id: i64 },
    "get_snapshot" => commands::snapshot::get_snapshot { id: i64 },
    "get_snapshot_pair_diff" => commands::snapshot::get_snapshot_pair_diff { before_id: Option<i64>, after_id: Option<i64> },
    "get_snapshot_summary" => commands::snapshot::get_snapshot_summary { snapshot_id: i64 },
    "restore_file_from_snapshot" => commands::snapshot::restore_file_from_snapshot { snapshot_id: i64 },
    // background
    "list_background_tasks" => commands::background::list_background_tasks {},
    "get_background_task" => commands::background::get_background_task { id: String },
    "start_background_task" => commands::background::start_background_task { kind: oxplow_app::BackgroundTaskKind, label: String, detail: Option<String> },
    "complete_background_task" => commands::background::complete_background_task { id: String, result_json: Option<String> },
    "fail_background_task" => commands::background::fail_background_task { id: String, error: String },
    "update_background_task" => commands::background::update_background_task { id: String, label: Option<String>, detail: Option<Option<String>>, progress: Option<Option<f64>> },
    // log
    "get_git_log" => commands::log::get_git_log { stream_id: Option<String>, limit: Option<u32>, all: bool },
    "get_commit_detail" => commands::log::get_commit_detail { stream_id: Option<String>, sha: String },
    "get_commits_ahead_of" => commands::log::get_commits_ahead_of { stream_id: Option<String>, base: String, head: String, limit: u32 },
    }
}

/// Shared helpers for dispatch round-trip tests. Crate-visible so each
/// `commands/<module>.rs` can write its own `#[cfg(test)]` round-trips
/// without duplicating the git-repo + Services scaffolding.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use oxplow_app::Services;

    /// Build an [`crate::RpcContext`] over an in-memory DB rooted at a
    /// fresh git repo (ensure_primary refuses non-git dirs). Returns a
    /// context with no plugin runtime — agent-spawn tests exercise the
    /// graceful-degradation path. Derefs to `Services`, so existing
    /// `svc.<store>` test usage keeps working. Keep the returned
    /// `TempDir` alive for the duration of the test.
    #[allow(clippy::unwrap_used)]
    pub fn services() -> (crate::RpcContext, tempfile::TempDir) {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "test"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let ctx = crate::RpcContext {
            services: Arc::new(Services::in_memory(dir.path()).unwrap()),
            plugin_runtime: None,
        };
        (ctx, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn ping_routes_with_no_args() {
        let (svc, _dir) = services();
        let out = dispatch("ping", json!(null), &svc).await.unwrap();
        assert_eq!(out, json!("pong"));
    }

    #[tokio::test]
    async fn list_streams_routes_and_serializes() {
        let (svc, _dir) = services();
        // Empty object body works the same as a null body for no-arg cmds.
        let out = dispatch("list_streams", json!({}), &svc).await.unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn get_task_deserializes_arg_and_returns_null_for_missing() {
        let (svc, _dir) = services();
        // A task id that doesn't exist → core returns None → JSON null.
        let out = dispatch("get_task", json!({ "id": "tsk999" }), &svc)
            .await
            .unwrap();
        assert_eq!(out, json!(null));
    }

    #[tokio::test]
    async fn unknown_command_is_not_found() {
        let (svc, _dir) = services();
        let err = dispatch("no_such_command", json!({}), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
    }

    #[tokio::test]
    async fn bad_args_are_rejected_as_invalid() {
        let (svc, _dir) = services();
        // `id` is required; an empty object can't deserialize into Args.
        let err = dispatch("get_task", json!({}), &svc).await.unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
