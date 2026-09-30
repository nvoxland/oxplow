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
pub mod envelope;
pub mod error;

pub use envelope::{ipc_envelope, ENVELOPE_CONTRACT};
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
    /// Base URL for the OTLP metrics receiver (epic tsk22) — pointed at via the
    /// agent's `OTEL_EXPORTER_OTLP_ENDPOINT`; the SDK appends `/v1/metrics`.
    pub otlp_base_url: String,
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
        gen { $( $gname:ident => $gcore:path { $( $gfield:ident : $gfty:ty ),* $(,)? } -> $gret:ty ),* $(,)? }
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
                $(
                    stringify!($gname) => {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args {
                            $( $gfield : $gfty ),*
                        }
                        let Args { $( $gfield ),* } = serde_json::from_value(args).map_err(|e| {
                            $crate::IpcError::invalid(format!("bad args for {name}: {e}"))
                        })?;
                        // The annotation is load-bearing: it pins the table's
                        // declared return type to the core's actual one, so a
                        // wrong row is a compile error rather than a silently
                        // wrong TS binding.
                        let out: $gret = $gcore(ctx.services.as_ref() $(, $gfield )*).await?;
                        serde_json::to_value(out).map_err(|e| {
                            $crate::IpcError::internal(format!("serialize result of {name}: {e}"))
                        })
                    }
                )*
                _ => Err($crate::IpcError::not_found()),
            }
        }

        /// Every command wire-name registered in this dispatch table.
        /// Tests use this to assert the remote-daemon registry stays in
        /// sync with the Tauri specta surface (a command that exists only
        /// as a Tauri adapter 404s under `POST /ipc/:name`).
        pub fn registered_command_names() -> &'static [&'static str] {
            &[ $( $cname, )* $( $name, )* $( stringify!($gname), )* ]
        }
    };
}

/// The one command table. Two callbacks expand it: `rpc_dispatch!`
/// here, and `tauri_adapters!` in oxplow-tauri-ipc — so a command is
/// declared once and both surfaces follow. Paths are absolute
/// (`$crate::`, `::oxplow_domain::`) because it expands in two crates
/// with different imports.
///
/// Rows in `gen` carry their core's return type and get a generated
/// Tauri adapter; rows still in `svc` keep a hand-written one. The
/// split exists so the migration can go module by module.
#[macro_export]
macro_rules! oxplow_command_table {
    ($callback:path) => {
        $callback! {
            ctx {
                // terminal — the agent-spawn path needs plugin_runtime
                "open_terminal_session" => $crate::commands::terminal::open_terminal_session { pane_target: String, cols: u16, rows: u16, transport_mode: String },
                // acp — opening a session needs plugin_runtime (oxplow's MCP endpoint)
                "acp_open_session" => $crate::commands::acp::acp_open_session { thread_id: ::oxplow_domain::ThreadId },
            }
            svc {
            }
            gen {
                // app
                ping => $crate::commands::app::ping {} -> &'static str,
                app_version => $crate::commands::app::app_version {} -> $crate::commands::app::AppVersion,
                log_ui => $crate::commands::app::log_ui { entry: $crate::commands::app::UiLogEntry } -> (),
                // streams
                // branch
                list_branches => $crate::commands::branch::list_branches {} -> Vec<::oxplow_git::BranchRef>,
                get_default_branch => $crate::commands::branch::get_default_branch {} -> Option<String>,
                rename_branch => $crate::commands::branch::rename_branch { from: String, to: String } -> (),
                delete_branch => $crate::commands::branch::delete_branch { branch: String, force: bool } -> (),
                list_local_branches => $crate::commands::branch::list_local_branches {} -> Vec<::oxplow_git::BranchRef>,
                // threads
                list_threads => $crate::commands::threads::list_threads { stream_id: ::oxplow_domain::StreamId } -> Vec<::oxplow_domain::Thread>,
                create_thread => $crate::commands::threads::create_thread { req: $crate::commands::threads::CreateThreadRequest } -> ::oxplow_domain::Thread,
                list_acp_agents => $crate::commands::threads::list_acp_agents {} -> Vec<::oxplow_app::acp::agents::AcpAgentListing>,
                // acp sessions (tsk281) — UI-only; acp_prompt is the prompt box's Enter
                acp_prompt => $crate::commands::acp::acp_prompt { thread_id: ::oxplow_domain::ThreadId, text: String } -> (),
                acp_cancel => $crate::commands::acp::acp_cancel { thread_id: ::oxplow_domain::ThreadId } -> (),
                acp_respond_permission => $crate::commands::acp::acp_respond_permission { thread_id: ::oxplow_domain::ThreadId, request_id: String, option_id: Option<String> } -> (),
                acp_transcript => $crate::commands::acp::acp_transcript { thread_id: ::oxplow_domain::ThreadId, since_seq: u64 } -> Option<::oxplow_app::acp::manager::AcpSnapshot>,
                acp_dismiss_directive => $crate::commands::acp::acp_dismiss_directive { thread_id: ::oxplow_domain::ThreadId } -> (),
                acp_close_session => $crate::commands::acp::acp_close_session { thread_id: ::oxplow_domain::ThreadId } -> (),
                rename_thread => $crate::commands::threads::rename_thread { req: $crate::commands::threads::RenameThreadRequest } -> ::oxplow_domain::Thread,
                set_thread_prompt => $crate::commands::threads::set_thread_prompt { req: $crate::commands::threads::SetThreadPromptRequest } -> ::oxplow_domain::Thread,
                promote_thread => $crate::commands::threads::promote_thread { id: ::oxplow_domain::ThreadId } -> ::oxplow_domain::Thread,
                close_thread => $crate::commands::threads::close_thread { id: ::oxplow_domain::ThreadId } -> ::oxplow_domain::Thread,
                reopen_thread => $crate::commands::threads::reopen_thread { id: ::oxplow_domain::ThreadId } -> ::oxplow_domain::Thread,
                list_closed_threads => $crate::commands::threads::list_closed_threads { stream_id: ::oxplow_domain::StreamId } -> Vec<::oxplow_domain::Thread>,
                reorder_thread_queue => $crate::commands::threads::reorder_thread_queue { req: $crate::commands::threads::ReorderThreadQueueRequest } -> (),
                get_thread_state => $crate::commands::threads::get_thread_state { stream_id: ::oxplow_domain::StreamId } -> $crate::commands::threads::ThreadState,
                get_thread_work_state => $crate::commands::threads::get_thread_work_state { thread_id: ::oxplow_domain::ThreadId } -> $crate::commands::threads::ThreadWorkState,
                select_thread => $crate::commands::threads::select_thread { req: $crate::commands::threads::SelectThreadRequest } -> (),
                // backlog
                list_backlog => $crate::commands::backlog::list_backlog {} -> Vec<::oxplow_domain::Task>,
                get_backlog_state => $crate::commands::backlog::get_backlog_state {} -> ::oxplow_app::BacklogState,
                // notes
                add_thread_note => $crate::commands::notes::add_thread_note { thread_id: ::oxplow_domain::ThreadId, body: String, author: String } -> ::oxplow_domain::TaskNote,
                list_thread_notes => $crate::commands::notes::list_thread_notes { thread_id: ::oxplow_domain::ThreadId } -> Vec<::oxplow_domain::TaskNote>,
                // tasks
                get_task => $crate::commands::tasks::get_task { id: ::oxplow_domain::TaskId } -> Option<::oxplow_domain::Task>,
                upsert_task => $crate::commands::tasks::upsert_task { item: ::oxplow_domain::Task } -> ::oxplow_domain::Task,
                delete_task => $crate::commands::tasks::delete_task { id: ::oxplow_domain::TaskId } -> (),
                create_task => $crate::commands::tasks::create_task { req: $crate::commands::tasks::CreateTaskRequest } -> ::oxplow_domain::Task,
                update_task => $crate::commands::tasks::update_task { req: $crate::commands::tasks::UpdateTaskRequest } -> ::oxplow_domain::Task,
                reorder_tasks => $crate::commands::tasks::reorder_tasks { req: $crate::commands::tasks::ReorderTasksRequest } -> (),
                move_task => $crate::commands::tasks::move_task { req: $crate::commands::tasks::MoveTaskRequest } -> ::oxplow_domain::Task,
                // dashboards (tsk138)
                list_dashboards => $crate::commands::dashboards::list_dashboards {} -> Vec<::oxplow_db::Dashboard>,
                get_dashboard => $crate::commands::dashboards::get_dashboard { id: ::oxplow_domain::DashboardId } -> Option<::oxplow_db::DashboardWithItems>,
                create_dashboard => $crate::commands::dashboards::create_dashboard { title: String } -> ::oxplow_db::Dashboard,
                rename_dashboard => $crate::commands::dashboards::rename_dashboard { req: $crate::commands::dashboards::RenameDashboardRequest } -> (),
                delete_dashboard => $crate::commands::dashboards::delete_dashboard { id: ::oxplow_domain::DashboardId } -> (),
                add_dashboard_item => $crate::commands::dashboards::add_dashboard_item { req: $crate::commands::dashboards::AddDashboardItemRequest } -> ::oxplow_domain::DashboardItemId,
                update_dashboard_item => $crate::commands::dashboards::update_dashboard_item { req: $crate::commands::dashboards::UpdateDashboardItemRequest } -> (),
                remove_dashboard_item => $crate::commands::dashboards::remove_dashboard_item { id: ::oxplow_domain::DashboardItemId } -> (),
                reorder_dashboard_items => $crate::commands::dashboards::reorder_dashboard_items { req: $crate::commands::dashboards::ReorderDashboardItemsRequest } -> (),
                // effort
                list_work_item_efforts => $crate::commands::effort::list_work_item_efforts { work_item: String } -> Vec<::oxplow_db::Effort>,
                list_efforts_in_window => $crate::commands::effort::list_efforts_in_window { window_start: ::oxplow_domain::Timestamp, window_end: ::oxplow_domain::Timestamp } -> Vec<::oxplow_db::Effort>,
                get_effort_files => $crate::commands::effort::get_effort_files { effort_id: ::oxplow_domain::EffortId } -> Vec<::oxplow_db::EffortFile>,
                get_effort => $crate::commands::effort::get_effort { effort_id: ::oxplow_domain::EffortId } -> Option<::oxplow_db::Effort>,
                list_efforts_at_snapshots => $crate::commands::effort::list_efforts_at_snapshots { snapshot_ids: Vec<i64> } -> Vec<::oxplow_db::EffortAtSnapshot>,
                list_efforts_overlapping_range => $crate::commands::effort::list_efforts_overlapping_range { range_start: i64, range_end: i64 } -> Vec<::oxplow_db::Effort>,
                list_changed_paths_for_effort => $crate::commands::effort::list_changed_paths_for_effort { effort_id: ::oxplow_domain::EffortId } -> ::oxplow_db::EffortChangedPaths,
                // metrics: reads are SQL (`metric_grid()`, `v_metric_catalog`); this
                // switches them, through the `metric.enable` command (P4.7)
                enable_metrics => $crate::commands::metrics::enable_metrics { keys: Vec<String>, enabled: bool } -> (),
                // followup
                list_followups => $crate::commands::followup::list_followups { thread_id: ::oxplow_domain::ThreadId } -> Vec<::oxplow_app::Followup>,
                add_followup => $crate::commands::followup::add_followup { thread_id: ::oxplow_domain::ThreadId, body: String } -> ::oxplow_app::Followup,
                remove_followup => $crate::commands::followup::remove_followup { id: String } -> (),
                // hooks
                ingest_hook_event => $crate::commands::hooks::ingest_hook_event { envelope: ::oxplow_app::HookEnvelope } -> (),
                list_agent_events => $crate::commands::hooks::list_agent_events { thread_id: Option<::oxplow_domain::ThreadId>, stream_id: Option<::oxplow_domain::StreamId>, limit: Option<usize> } -> Vec<::oxplow_domain::StoredEvent>,
                read_event_content => $crate::commands::hooks::read_event_content { event_id: String, body: oxplow_app::event_bodies::EventBodyKey } -> Option<oxplow_app::event_bodies::EventBody>,
                list_agent_statuses => $crate::commands::hooks::list_agent_statuses {} -> Vec<::oxplow_domain::AgentStatus>,
                list_open_agent_turns => $crate::commands::hooks::list_open_agent_turns { thread_id: ::oxplow_domain::ThreadId } -> Vec<::oxplow_domain::AgentTurn>,
                get_agent_turn => $crate::commands::hooks::get_agent_turn { turn_id: ::oxplow_domain::AgentTurnId } -> Option<::oxplow_domain::AgentTurn>,
                // wiki
                list_wiki_pages => $crate::commands::wiki::list_wiki_pages {} -> Vec<::oxplow_db::WikiPage>,
                upsert_wiki_page => $crate::commands::wiki::upsert_wiki_page { note: ::oxplow_db::WikiPage } -> (),
                delete_wiki_page => $crate::commands::wiki::delete_wiki_page { slug: String } -> (),
                search_wiki_titles => $crate::commands::wiki::search_wiki_titles { query: String, limit: u32 } -> Vec<::oxplow_db::WikiPage>,
                read_wiki_page_body => $crate::commands::wiki::read_wiki_page_body { slug: String } -> String,
                write_wiki_page_body => $crate::commands::wiki::write_wiki_page_body { slug: String, body: String } -> (),
                // events (the log's dead-letter queue)
                list_dead_letters => $crate::commands::events::list_dead_letters { all: Option<bool> } -> Vec<::oxplow_db::DeadLetter>,
                retry_dead_letter => $crate::commands::events::retry_dead_letter { id: i64 } -> ::oxplow_db::DeadLetter,
                discard_dead_letter => $crate::commands::events::discard_dead_letter { id: i64 } -> ::oxplow_db::DeadLetter,
                // page_refs
                list_backlinks => $crate::commands::page_refs::list_backlinks { target_kind: String, target_id: String, limit: Option<i64> } -> Vec<$crate::commands::page_refs::BacklinkEdge>,
                list_outbound => $crate::commands::page_refs::list_outbound { source_kind: String, source_id: String, limit: Option<i64> } -> Vec<$crate::commands::page_refs::BacklinkEdge>,
                // wiki_freshness
                list_wiki_freshness => $crate::commands::wiki_freshness::list_wiki_freshness { slug: String } -> Vec<$crate::commands::wiki_freshness::WikiRefFreshness>,
                mark_wiki_ref_verified => $crate::commands::wiki_freshness::mark_wiki_ref_verified { slug: String, path: String } -> (),
                mark_all_wiki_refs_verified => $crate::commands::wiki_freshness::mark_all_wiki_refs_verified { slug: String } -> usize,
                // search
                search => $crate::commands::search::search { query: String, stream_id: Option<String>, kinds: Option<Vec<String>>, limit: Option<u32> } -> Vec<::oxplow_db::SearchHit>,
                query_sql => $crate::commands::semantic::query_sql { sql: String, params: Option<Vec<::oxplow_db::SqlCell>>, limit: Option<u32>, raw: Option<bool> } -> ::oxplow_db::SqlQueryResult,
                describe_schema => $crate::commands::semantic::describe_schema {} -> Vec<::oxplow_db::SchemaEntity>,
                semantic_row_counts => $crate::commands::semantic::semantic_row_counts {} -> Vec<::oxplow_app::semantic_catalog::EntityRowCount>,
                list_extensions => $crate::commands::extensions::list_extensions { stream_id: Option<String> } -> Vec<::oxplow_app::extensions::Extension>,
                get_lens => $crate::commands::extensions::get_lens { id: String, stream_id: Option<String> } -> ::oxplow_app::extensions::Lens,
                run_lens => $crate::commands::extensions::run_lens { id: String, params: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>>, stream_id: Option<String> } -> ::oxplow_app::extensions::LensRun,
                run_lens_action => $crate::commands::extensions::run_lens_action { id: String, action: String, params: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>>, stream_id: Option<String> } -> ::oxplow_app::lens_actions::LensActionResult,
                validate_extension => $crate::commands::extensions::validate_extension { name: String, stream_id: Option<String> } -> ::oxplow_sdk::CheckReport,
                review_extension => $crate::commands::extensions::review_extension { git_url: Option<String>, git_ref: Option<String>, name: Option<String>, stream_id: Option<String> } -> ::oxplow_app::extensions::ExtensionReview,
                install_extension => $crate::commands::extensions::install_extension { git_url: String, git_ref: Option<String>, reviewed_sha: String, stream_id: Option<String> } -> ::oxplow_app::extensions::Extension,
                update_extension => $crate::commands::extensions::update_extension { name: String, reviewed_sha: String, stream_id: Option<String> } -> ::oxplow_app::extensions::Extension,
                set_extension_enabled => $crate::commands::extensions::set_extension_enabled { name: String, enabled: bool } -> Vec<::oxplow_app::extensions::Extension>,
                list_sources => $crate::commands::sources::list_sources {} -> Vec<::oxplow_app::source_runner::SourceListing>,
                ensure_change => $crate::commands::changes::ensure_change { target: ::oxplow_app::change_analysis::ChangeTarget } -> ::oxplow_db::ChangeRow,
                // ai (Settings → AI)
                ai_settings => $crate::commands::ai::ai_settings {} -> ::oxplow_app::ai_service::AiSettings,
                save_ai_provider => $crate::commands::ai::save_ai_provider { provider: ::oxplow_app::ai_service::ProviderConfig, key: Option<String> } -> ::oxplow_app::ai_service::AiSettings,
                remove_ai_provider => $crate::commands::ai::remove_ai_provider { id: String } -> ::oxplow_app::ai_service::AiSettings,
                set_ai_role => $crate::commands::ai::set_ai_role { role: ::oxplow_app::ai_service::Role, binding: Option<::oxplow_app::ai_service::RoleBinding> } -> ::oxplow_app::ai_service::AiSettings,
                test_ai_provider => $crate::commands::ai::test_ai_provider { id: String, model: String } -> String,
                run_source => $crate::commands::sources::run_source { extension: String, source_id: String, approve: Option<String> } -> ::oxplow_app::source_runner::SourceRunReport,
                set_source_credential => $crate::commands::sources::set_source_credential { extension: String, name: String, value: Option<String> } -> (),
                list_project_programs => $crate::commands::sources::list_project_programs {} -> Vec<::oxplow_app::exec_consent::ProjectProgram>,
                approve_project_program => $crate::commands::sources::approve_project_program { kind: ::oxplow_app::exec_consent::ProgramKind, name: String, version: String } -> Vec<::oxplow_app::exec_consent::ProjectProgram>,
                report_open_page => $crate::commands::open_page::report_open_page { thread_id: String, page_id: Option<String>, kind: Option<String>, detail_json: Option<String> } -> (),
                save_lens => $crate::commands::extensions::save_lens { extension: String, slug: String, lens: ::oxplow_app::extensions::NewLens, stream_id: Option<String> } -> ::oxplow_app::extensions::Lens,
                // comments
                create_comment => $crate::commands::comments::create_comment { req: $crate::commands::comments::CreateCommentRequest } -> ::oxplow_domain::CommentThread,
                add_comment_message => $crate::commands::comments::add_comment_message { comment_id: ::oxplow_domain::CommentId, author: String, body: String } -> ::oxplow_domain::CommentMessage,
                list_comments_for_target => $crate::commands::comments::list_comments_for_target { target_kind: String, target_id: String } -> Vec<::oxplow_domain::CommentThread>,
                list_comments_for_stream => $crate::commands::comments::list_comments_for_stream { stream_id: ::oxplow_domain::StreamId } -> Vec<::oxplow_domain::CommentThread>,
                set_comment_intent => $crate::commands::comments::set_comment_intent { comment_id: ::oxplow_domain::CommentId, intent: ::oxplow_domain::CommentIntent } -> (),
                set_comment_status => $crate::commands::comments::set_comment_status { comment_id: ::oxplow_domain::CommentId, status: ::oxplow_domain::CommentStatus } -> (),
                set_comment_anchor => $crate::commands::comments::set_comment_anchor { comment_id: ::oxplow_domain::CommentId, selectors_json: String, orphaned: bool } -> (),
                relink_comment => $crate::commands::comments::relink_comment { comment_id: ::oxplow_domain::CommentId, quote: String, selectors_json: String } -> (),
                delete_comment => $crate::commands::comments::delete_comment { comment_id: ::oxplow_domain::CommentId } -> (),
                // page_visit
                record_page_visit => $crate::commands::page_visit::record_page_visit { page_kind: String, page_id: String, label: Option<String>, duration_ms: Option<i64>, thread_id: Option<String> } -> ::oxplow_db::PageVisit,
                list_recent_page_visits => $crate::commands::page_visit::list_recent_page_visits { limit: u32, thread_id: Option<String> } -> Vec<::oxplow_db::PageVisit>,
                top_visited_pages => $crate::commands::page_visit::top_visited_pages { limit: u32, thread_id: Option<String> } -> Vec<$crate::commands::page_visit::VisitedPage>,
                forget_page => $crate::commands::page_visit::forget_page { page_kind: String, page_id: String } -> (),
                list_recently_finished => $crate::commands::page_visit::list_recently_finished { thread_id: Option<String>, limit: u32 } -> Vec<$crate::commands::page_visit::FinishedEntry>,
                clear_recently_finished => $crate::commands::page_visit::clear_recently_finished { thread_id: Option<String> } -> (),
                // usage
                record_usage => $crate::commands::usage::record_usage { kind: String, payload_json: String } -> ::oxplow_db::UsageEvent,
                list_recent_usage_rollup => $crate::commands::usage::list_recent_usage_rollup { kind: String, stream_id: Option<String>, limit: u32 } -> Vec<::oxplow_db::UsageRollup>,
                // git
                get_repo_conflict_state => $crate::commands::git::get_repo_conflict_state { stream_id: Option<String> } -> ::oxplow_git::RepoConflictState,
                get_ahead_behind => $crate::commands::git::get_ahead_behind { stream_id: Option<String>, base: String, head: String } -> ::oxplow_git::AheadBehind,
                list_stream_divergences => $crate::commands::git::list_stream_divergences { base: Option<String> } -> $crate::commands::git::StreamDivergenceReport,
                append_to_gitignore => $crate::commands::git::append_to_gitignore { stream_id: Option<String>, entry: String } -> (),
                restore_path => $crate::commands::git::restore_path { stream_id: Option<String>, path: String } -> (),
                git_fetch => $crate::commands::git::git_fetch { stream_id: Option<String>, remote: Option<String> } -> ::oxplow_git::GitOpResult,
                git_pull => $crate::commands::git::git_pull { stream_id: Option<String> } -> ::oxplow_git::GitOpResult,
                git_pull_remote_into_current => $crate::commands::git::git_pull_remote_into_current { stream_id: Option<String>, remote: String, branch: String } -> ::oxplow_git::GitOpResult,
                git_push => $crate::commands::git::git_push { stream_id: Option<String> } -> ::oxplow_git::GitOpResult,
                git_push_current_to => $crate::commands::git::git_push_current_to { stream_id: Option<String>, remote: String, branch: String } -> ::oxplow_git::GitOpResult,
                git_merge_into => $crate::commands::git::git_merge_into { stream_id: Option<String>, source: String } -> ::oxplow_git::GitOpResult,
                git_rebase_onto => $crate::commands::git::git_rebase_onto { stream_id: Option<String>, onto: String } -> ::oxplow_git::GitOpResult,
                git_cherry_pick => $crate::commands::git::git_cherry_pick { stream_id: Option<String>, commit: String } -> ::oxplow_git::GitOpResult,
                git_revert => $crate::commands::git::git_revert { stream_id: Option<String>, commit: String } -> ::oxplow_git::GitOpResult,
                git_commit_all => $crate::commands::git::git_commit_all { stream_id: Option<String>, message: String } -> ::oxplow_git::GitOpResult,
                git_add_path => $crate::commands::git::git_add_path { stream_id: Option<String>, path: String } -> ::oxplow_git::GitOpResult,
                list_all_refs => $crate::commands::git::list_all_refs {} -> ::oxplow_git::GroupedGitRefs,
                resolve_commit_ref_labels => $crate::commands::git::resolve_commit_ref_labels { shas: Vec<String> } -> ::std::collections::HashMap<String, Vec<::oxplow_git::CommitRefLabel>>,
                list_recent_remote_branches => $crate::commands::git::list_recent_remote_branches { limit: Option<usize> } -> Vec<::oxplow_git::RemoteBranchEntry>,
                list_file_commits => $crate::commands::git::list_file_commits { stream_id: Option<String>, path: String, limit: Option<usize> } -> Vec<oxplow_git::GitLogCommit>,
                git_blame => $crate::commands::git::git_blame { stream_id: Option<String>, path: String } -> Vec<::oxplow_git::BlameLine>,
                local_blame => $crate::commands::git::local_blame { stream_id: Option<String>, path: String, disk_text: String } -> Vec<::oxplow_git::LocalBlameEntry>,
                get_change_scopes => $crate::commands::git::get_change_scopes { stream_id: Option<String> } -> ::oxplow_git::ChangeScopes,
                get_branch_changes => $crate::commands::git::get_branch_changes { stream_id: Option<String>, base_ref: String } -> ::oxplow_git::BranchChanges,
                list_adoptable_worktrees => $crate::commands::git::list_adoptable_worktrees {} -> Vec<::oxplow_git::GitWorktreeEntry>,
                search_workspace_text => $crate::commands::git::search_workspace_text { stream_id: Option<String>, query: String, limit: Option<usize> } -> Vec<::oxplow_git::TextSearchHit>,
                read_file_at_ref => $crate::commands::git::read_file_at_ref { r#ref: String, path: String } -> Option<String>,
                // workspace
                read_file => $crate::commands::workspace::read_file { stream_id: Option<String>, relative_path: String, version: ::oxplow_tree_source::TreeVersion } -> Option<String>,
                list_workspace_entries => $crate::commands::workspace::list_workspace_entries { stream_id: Option<String>, relative_path: String } -> Vec<::oxplow_git::WorkspaceEntry>,
                list_workspace_files => $crate::commands::workspace::list_workspace_files { stream_id: Option<String> } -> Vec<::oxplow_git::WorkspaceIndexedFile>,
                read_workspace_file => $crate::commands::workspace::read_workspace_file { stream_id: Option<String>, relative_path: String } -> ::oxplow_git::WorkspaceFile,
                write_workspace_file => $crate::commands::workspace::write_workspace_file { stream_id: Option<String>, relative_path: String, content: String } -> ::oxplow_git::WorkspaceFile,
                create_workspace_file => $crate::commands::workspace::create_workspace_file { stream_id: Option<String>, relative_path: String, content: String } -> ::oxplow_git::WorkspaceFile,
                create_workspace_directory => $crate::commands::workspace::create_workspace_directory { stream_id: Option<String>, relative_path: String } -> String,
                rename_workspace_path => $crate::commands::workspace::rename_workspace_path { stream_id: Option<String>, from_path: String, to_path: String } -> (String, String),
                delete_workspace_path => $crate::commands::workspace::delete_workspace_path { stream_id: Option<String>, relative_path: String } -> String,
                get_workspace_status_summary => $crate::commands::workspace::get_workspace_status_summary { stream_id: Option<String> } -> ::oxplow_git::WorkspaceStatusSummary,
                // code_quality (duplication only — the metrics scan was retired in tsk229)
                // config
                get_config => $crate::commands::config::get_config {} -> ::oxplow_config::OxplowConfig,
                set_agent_prompt_append => $crate::commands::config::set_agent_prompt_append { text: String } -> ::oxplow_config::OxplowConfig,
                set_agents => $crate::commands::config::set_agents { agents: Vec<::oxplow_config::AgentKind> } -> ::oxplow_config::OxplowConfig,
                set_snapshot_retention_days => $crate::commands::config::set_snapshot_retention_days { days: u32 } -> ::oxplow_config::OxplowConfig,
                set_snapshot_max_file_bytes => $crate::commands::config::set_snapshot_max_file_bytes { bytes: u64 } -> ::oxplow_config::OxplowConfig,
                set_generated => $crate::commands::config::set_generated { generated: ::oxplow_config::GeneratedConfig } -> ::oxplow_config::OxplowConfig,
                set_agent_model => $crate::commands::config::set_agent_model { agent: ::oxplow_config::AgentKind, model: Option<String> } -> ::oxplow_config::OxplowConfig,
                get_workspace_context => $crate::commands::config::get_workspace_context {} -> $crate::commands::config::WorkspaceContext,
                // lsp
                install_lsp_package => $crate::commands::lsp::install_lsp_package { package_name: String } -> $crate::commands::lsp::InstalledLspPackage,
                list_installed_lsp_packages => $crate::commands::lsp::list_installed_lsp_packages {} -> Vec<$crate::commands::lsp::InstalledLspPackage>,
                lsp_request => $crate::commands::lsp::lsp_request { stream_id: String, language_id: String, method: String, params_json: String } -> String,
                lsp_notify => $crate::commands::lsp::lsp_notify { stream_id: String, language_id: String, method: String, params_json: String } -> (),
                list_lsp_servers => $crate::commands::lsp::list_lsp_servers {} -> Vec<::oxplow_app::lsp_sessions::LspServerListing>,
                restart_lsp_server => $crate::commands::lsp::restart_lsp_server { stream_id: String, language_id: String } -> (),
                remove_lsp_package => $crate::commands::lsp::remove_lsp_package { package_name: String } -> (),
                respond_lsp_apply_edit => $crate::commands::lsp::respond_lsp_apply_edit { token: u32, applied: bool, failure_reason: Option<String> } -> (),
                // terminal (open_terminal_session stays Tauri-only: PluginRuntimeState)
                forward_terminal_input => $crate::commands::terminal::forward_terminal_input { session_id: String, message: String } -> (),
                close_terminal_session => $crate::commands::terminal::close_terminal_session { session_id: String } -> (),
                terminal_session_cwd => $crate::commands::terminal::terminal_session_cwd { session_id: String } -> Option<String>,
                terminate_terminal_session => $crate::commands::terminal::terminate_terminal_session { session_id: String } -> (),
                lookup_terminal_session => $crate::commands::terminal::lookup_terminal_session { thread_id: ::oxplow_domain::ThreadId, pane: Option<String> } -> Option<String>,
                // snapshot
                list_file_snapshots => $crate::commands::snapshot::list_file_snapshots { path: String } -> Vec<::oxplow_db::FileSnapshot>,
                list_snapshots_for_stream => $crate::commands::snapshot::list_snapshots_for_stream { stream_id: ::oxplow_domain::StreamId, limit: Option<usize> } -> Vec<::oxplow_db::Snapshot>,
                list_snapshot_ops => $crate::commands::snapshot::list_snapshot_ops { stream_id: ::oxplow_domain::StreamId, limit: Option<usize> } -> Vec<::oxplow_db::SnapshotOp>,
                get_snapshot_stats => $crate::commands::snapshot::get_snapshot_stats { snapshot_id: i64 } -> ::oxplow_db::SnapshotStats,
                get_blob_storage_bytes => $crate::commands::snapshot::get_blob_storage_bytes {} -> i64,
                list_wiki_slugs_for_snapshots => $crate::commands::snapshot::list_wiki_slugs_for_snapshots { snapshot_ids: Vec<i64> } -> Vec<(i64, String)>,
                list_files_for_snapshot => $crate::commands::snapshot::list_files_for_snapshot { snapshot_id: i64 } -> Vec<::oxplow_db::FileSnapshot>,
                get_file_snapshot => $crate::commands::snapshot::get_file_snapshot { file_snapshot_id: i64 } -> Option<::oxplow_db::FileSnapshot>,
                diff_endpoints => $crate::commands::snapshot::diff_endpoints { start: Option<$crate::commands::snapshot::DiffEndpoint>, end: $crate::commands::snapshot::DiffEndpoint } -> Vec<$crate::commands::snapshot::DiffEntry>,
                restore_file_snapshot => $crate::commands::snapshot::restore_file_snapshot { file_snapshot_id: i64 } -> (),
                // background
                list_background_tasks => $crate::commands::background::list_background_tasks {} -> Vec<::oxplow_app::BackgroundTask>,
                get_background_task => $crate::commands::background::get_background_task { id: String } -> Option<::oxplow_app::BackgroundTask>,
                start_background_task => $crate::commands::background::start_background_task { kind: ::oxplow_app::BackgroundTaskKind, label: String, detail: Option<String> } -> ::oxplow_app::BackgroundTask,
                complete_background_task => $crate::commands::background::complete_background_task { id: String, result_json: Option<String> } -> (),
                fail_background_task => $crate::commands::background::fail_background_task { id: String, error: String } -> (),
                update_background_task => $crate::commands::background::update_background_task { id: String, label: Option<String>, detail: Option<Option<String>>, progress: Option<Option<f64>> } -> (),
                // log
                get_git_log => $crate::commands::log::get_git_log { stream_id: Option<String>, limit: Option<u32>, all: bool } -> ::oxplow_git::GitLogResult,
                get_commit_detail => $crate::commands::log::get_commit_detail { stream_id: Option<String>, sha: String } -> Option<::oxplow_git::CommitDetail>,
                get_commits_ahead_of => $crate::commands::log::get_commits_ahead_of { stream_id: Option<String>, base: String, head: String, limit: u32 } -> Vec<::oxplow_git::GitLogCommit>,

                // streams
                list_streams => $crate::commands::streams::list_streams {} -> ::std::vec::Vec<::oxplow_domain::Stream>,
                create_worktree => $crate::commands::streams::create_worktree { req: $crate::commands::streams::CreateWorktreeRequest } -> ::oxplow_domain::Stream,
                adopt_worktree => $crate::commands::streams::adopt_worktree { req: $crate::commands::streams::AdoptWorktreeRequest } -> ::oxplow_domain::Stream,
                archive_stream => $crate::commands::streams::archive_stream { id: ::oxplow_domain::StreamId, delete_worktree: bool } -> (),
                get_primary_stream => $crate::commands::streams::get_primary_stream {} -> Option<::oxplow_domain::Stream>,
                get_current_stream => $crate::commands::streams::get_current_stream {} -> Option<::oxplow_domain::Stream>,
                switch_stream => $crate::commands::streams::switch_stream { id: Option<::oxplow_domain::StreamId> } -> (),
                rename_stream => $crate::commands::streams::rename_stream { req: $crate::commands::streams::RenameStreamRequest } -> ::oxplow_domain::Stream,
                set_stream_prompt => $crate::commands::streams::set_stream_prompt { req: $crate::commands::streams::SetStreamPromptRequest } -> ::oxplow_domain::Stream,
                reorder_streams => $crate::commands::streams::reorder_streams { order: Vec<::oxplow_domain::StreamId> } -> (),
                checkout_stream_branch => $crate::commands::streams::checkout_stream_branch { id: ::oxplow_domain::StreamId, branch: String } -> ::oxplow_domain::Stream,
            }
        }
    };
}

oxplow_command_table!(rpc_dispatch);

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
