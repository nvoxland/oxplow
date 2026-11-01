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

/// Control-plane coordinates an agent spawn needs: where the Claude Code plugin's
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
                        // Boxed, like each arm: dispatch holds one arm's future at a
                        // time, so none may size the whole state machine.
                        let out = Box::pin($ccore(ctx $(, $cfield )*)).await?;
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
                        let out = Box::pin($core(ctx.services.as_ref() $(, $field )*)).await?;
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
                        let out: $gret = Box::pin($gcore(ctx.services.as_ref() $(, $gfield )*)).await?;
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
                "open_terminal_session" => $crate::commands::terminal::open_terminal_session { uri: String, cols: u16, rows: u16 },
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
                // threads
                list_threads => $crate::commands::threads::list_threads { stream_id: ::oxplow_domain::StreamId } -> Vec<::oxplow_domain::Thread>,
                list_agent_harnesses => $crate::commands::threads::list_agent_harnesses {} -> Vec<::oxplow_app::harnesses::HarnessListing>,
                get_thread_state => $crate::commands::threads::get_thread_state { stream_id: ::oxplow_domain::StreamId } -> $crate::commands::threads::ThreadState,
                select_thread => $crate::commands::threads::select_thread { req: $crate::commands::threads::SelectThreadRequest } -> (),
                // backlog
                // tasks
                // effort
                list_efforts_in_window => $crate::commands::effort::list_efforts_in_window { window_start: ::oxplow_domain::Timestamp, window_end: ::oxplow_domain::Timestamp } -> Vec<::oxplow_db::Effort>,
                list_efforts_at_snapshots => $crate::commands::effort::list_efforts_at_snapshots { snapshot_ids: Vec<::oxplow_domain::SnapshotId> } -> Vec<::oxplow_db::EffortAtSnapshot>,
                // metrics: reads are SQL (`metric_grid()`, `v_metric_catalog`); this
                // switches them, through the `oxplow.metric.enable` command (P4.7)
                // the command bus, as the person
                get_command => $crate::commands::bus::get_command { name: String } -> ::oxplow_domain::CommandSpec,
                list_person_commands => $crate::commands::bus::list_person_commands {} -> Vec<::oxplow_domain::CommandSpec>,
                ref_commands => $crate::commands::bus::ref_commands { uri: String } -> Vec<String>,
                register_client_host => $crate::commands::bus::register_client_host { client: String, scopes: Vec<String> } -> (),
                unregister_client_host => $crate::commands::bus::unregister_client_host { client: String } -> (),
                answer_client_call => $crate::commands::bus::answer_client_call { client: String, id: String, result: Option<::oxplow_domain::Json>, error: Option<String> } -> (),
                run_command_for_call => $crate::commands::bus::run_command_for_call { client: String, call: String, id: String, input: ::oxplow_domain::Json } -> ::oxplow_domain::CommandOutcome,
                run_command => $crate::commands::bus::run_command { id: String, input: ::oxplow_domain::Json, confirmed: bool } -> ::oxplow_domain::CommandOutcome,
                command_open_after => $crate::commands::bus::command_open_after { id: String, result: ::oxplow_domain::Json } -> Option<String>,
                undo_command => $crate::commands::bus::undo_command { audit_id: ::oxplow_domain::AuditId, confirmed: bool } -> ::oxplow_domain::CommandOutcome,
                decide_proposal => $crate::commands::bus::decide_proposal { proposal: ::oxplow_domain::ProposalId, approve: bool } -> Option<::oxplow_domain::CommandOutcome>,
                // hooks
                ingest_hook_event => $crate::commands::hooks::ingest_hook_event { envelope: ::oxplow_app::HookEnvelope } -> (),
                read_event_content => $crate::commands::hooks::read_event_content { event_id: String, body: oxplow_app::event_bodies::EventBodyKey } -> Option<oxplow_app::event_bodies::EventBody>,
                list_agent_statuses => $crate::commands::hooks::list_agent_statuses {} -> Vec<::oxplow_domain::AgentStatus>,
                // wiki
                // events (the log's dead-letter queue)
                // page_refs
                list_backlinks => $crate::commands::page_refs::list_backlinks { target: String, limit: Option<u32> } -> Vec<$crate::commands::page_refs::BacklinkEdge>,
                list_outbound => $crate::commands::page_refs::list_outbound { source: String, limit: Option<u32> } -> Vec<$crate::commands::page_refs::BacklinkEdge>,
                // wiki_freshness
                // search
                search => $crate::commands::search::search { query: String, stream_id: Option<String>, resources: Option<Vec<String>>, limit: Option<u32> } -> Vec<::oxplow_db::SearchHit>,
                query_sql => $crate::commands::semantic::query_sql { sql: String, params: Option<Vec<::oxplow_db::SqlCell>>, limit: Option<u32>, raw: Option<bool> } -> ::oxplow_db::SqlQueryResult,
                get_panel_layout => $crate::commands::semantic::get_panel_layout {} -> Vec<::oxplow_db::PanelPlacement>,
                set_panel_layout => $crate::commands::semantic::set_panel_layout { layout: Vec<::oxplow_db::PanelPlacement> } -> (),
                get_ui_preferences => $crate::commands::semantic::get_ui_preferences {} -> Vec<::oxplow_db::UiPreference>,
                set_ui_preference => $crate::commands::semantic::set_ui_preference { scope: String, key: String, value_json: Option<String> } -> (),
                list_extensions => $crate::commands::extensions::list_extensions {} -> Vec<::oxplow_app::extension::extensions::Extension>,
                load_component => $crate::commands::extensions::load_component { id: String } -> String,
                invoke_component_command => $crate::commands::extensions::invoke_component_command { id: String, command: String, input: ::oxplow_domain::Json, confirmed: bool, version: String } -> ::oxplow_domain::CommandOutcome,
                run_component_sql => $crate::commands::extensions::run_component_sql { id: String, sql: String, params: Option<Vec<::oxplow_db::SqlCell>>, stream_id: Option<String>, version: String } -> ::oxplow_db::SqlQueryResult,
                component_form => $crate::commands::extensions::component_form { id: String, command: String, input: Option<::oxplow_domain::Json>, version: String } -> ::oxplow_app::viz_actions::FormStart,
                list_services => $crate::commands::extensions::list_services {} -> Vec<$crate::commands::extensions::ServiceListing>,
                // pages: a window's open runs
                open_page => $crate::commands::pages::open_page { page: Option<String>, spec_yaml: Option<String>, params: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>>, stream_id: Option<String>, thread_id: Option<String>, client: String } -> $crate::commands::pages::OpenedRun,
                close_page => $crate::commands::pages::close_page { run_id: String } -> (),
                set_page_params => $crate::commands::pages::set_page_params { run_id: String, params: ::std::collections::BTreeMap<String, ::oxplow_db::SqlCell> } -> ::oxplow_app::pages::runner::PageRun,
                rerun_page => $crate::commands::pages::rerun_page { run_id: String } -> ::oxplow_app::pages::runner::PageRun,
                set_page_visible => $crate::commands::pages::set_page_visible { run_id: String, visible: bool } -> bool,
                fetch_page_runs => $crate::commands::pages::fetch_page_runs { run_ids: Vec<String> } -> ::std::collections::BTreeMap<String, ::oxplow_app::pages::open_runs::RunSnapshot>,
                run_page => $crate::commands::pages::run_page { page: Option<String>, spec_yaml: Option<String>, params: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>>, stream_id: Option<String> } -> ::oxplow_app::pages::runner::PageRun,
                page_text => $crate::commands::pages::page_text { run_id: String } -> String,
                get_page => $crate::commands::pages::get_page { id: String } -> ::oxplow_app::pages::schema::Page,
                renderers_for => $crate::commands::pages::renderers_for { uri: String, stream_id: Option<String> } -> Vec<::oxplow_app::pages::renderers::Renderer>,
                page_spec => $crate::commands::pages::page_spec { page: String } -> $crate::commands::pages::PageSpec,
                page_form => $crate::commands::pages::page_form { run_id: String, node: String } -> ::oxplow_app::viz_actions::FormStart,
                submit_page_form => $crate::commands::pages::submit_page_form { run_id: String, node: String, input: ::oxplow_domain::Json, confirmed: bool } -> ::oxplow_domain::CommandOutcome,
                page_action_form => $crate::commands::pages::page_action_form { run_id: String, node: Option<String>, action: String, row: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>> } -> ::oxplow_app::viz_actions::FormStart,
                run_page_action => $crate::commands::pages::run_page_action { run_id: String, node: Option<String>, action: String, row: Option<::std::collections::BTreeMap<String, ::oxplow_db::SqlCell>>, input: Option<::oxplow_domain::Json>, drop: Option<::oxplow_app::viz_actions::Drop>, confirmed: bool } -> ::oxplow_domain::CommandOutcome,
                // ai (Settings → AI)
                ai_settings => $crate::commands::ai::ai_settings {} -> ::oxplow_app::ai_service::AiSettings,
                read_ai_call => $crate::commands::ai::read_ai_call { id: ::oxplow_domain::AiCallId, body: ::oxplow_app::ai_calls::AiCallBody } -> Option<::oxplow_app::event_bodies::EventBody>,
                // the Assistant's panel (UI-only)
                assistant_open => $crate::commands::assistant::assistant_open { stream: ::oxplow_domain::StreamId, thread: Option<::oxplow_domain::ThreadId>, context: Option<::oxplow_app::assistant::AssistantContext> } -> ::oxplow_app::assistant::AssistantOpened,
                assistant_prompt => $crate::commands::assistant::assistant_prompt { id: String, text: String } -> (),
                assistant_cancel => $crate::commands::assistant::assistant_cancel { id: String } -> (),
                assistant_set => $crate::commands::assistant::assistant_set { id: String, dataset: Option<::oxplow_app::assistant::AssistantModelChoice>, allow_changes: Option<bool> } -> (),
                assistant_answer => $crate::commands::assistant::assistant_answer { id: String, ask: String, allow: bool } -> (),
                assistant_snapshot => $crate::commands::assistant::assistant_snapshot { id: String, since: u32 } -> ::oxplow_app::assistant::AssistantSnapshot,
                assistant_handoff => $crate::commands::assistant::assistant_handoff { id: String, to_thread: String } -> ::oxplow_domain::Json,
                assistant_close => $crate::commands::assistant::assistant_close { id: String } -> (),
                report_tabs => $crate::commands::tabs::report_tabs { report: oxplow_app::client_reports::TabReport } -> (),
                // comments
                list_comments_for_target => $crate::commands::comments::list_comments_for_target { target: String } -> Vec<::oxplow_domain::CommentThread>,
                list_comments_for_stream => $crate::commands::comments::list_comments_for_stream { stream_id: ::oxplow_domain::StreamId } -> Vec<::oxplow_domain::CommentThread>,
                // page_visit
                record_page_visit => $crate::commands::page_visit::record_page_visit { page_kind: String, page_id: String, label: Option<String>, duration_ms: Option<u32>, thread_id: Option<String> } -> ::oxplow_db::PageVisit,
                list_recent_page_visits => $crate::commands::page_visit::list_recent_page_visits { limit: u32, thread_id: Option<String> } -> Vec<::oxplow_db::PageVisit>,
                top_visited_pages => $crate::commands::page_visit::top_visited_pages { limit: u32, thread_id: Option<String> } -> Vec<$crate::commands::page_visit::VisitedPage>,
                forget_page => $crate::commands::page_visit::forget_page { page_kind: String, page_id: String } -> (),
                // usage
                record_usage => $crate::commands::usage::record_usage { kind: String, payload_json: String } -> ::oxplow_db::UsageEvent,
                // git
                git_resolve_commit_ref_labels => $crate::commands::git::git_resolve_commit_ref_labels { shas: Vec<String> } -> ::std::collections::HashMap<String, Vec<::oxplow_app::vcs::CommitRefLabel>>,
                read_at => $crate::commands::trees::read_at { stream_id: Option<String>, path: String, filesystem: Option<String>, rev: Option<String> } -> Option<String>,
                stat_file => $crate::commands::trees::stat_file { stream_id: Option<String>, path: String, filesystem: Option<String>, rev: Option<String> } -> Option<::oxplow_domain::filesystem::FsStat>,
                // workspace
                resolve_symbol => $crate::commands::trees::resolve_symbol { stream_id: String, symbol: String } -> Option<::oxplow_app::symbol_resolve::SymbolLocation>,
                files_at => $crate::commands::trees::files_at { stream_id: Option<String>, revision: ::oxplow_domain::vcs::Revision } -> Vec<String>,
                vcs_blame => $crate::commands::vcs::vcs_blame { stream_id: Option<String>, path: String, revision: ::oxplow_domain::vcs::Revision } -> Vec<::oxplow_domain::vcs::BlameLine>,
                read_workspace_file => $crate::commands::workspace::read_workspace_file { stream_id: Option<String>, relative_path: String } -> ::oxplow_app::workspace_files::WorkspaceFile,
                // code_quality (duplication only — the metrics scan was retired in tsk229)
                // config
                get_config => $crate::commands::config::get_config {} -> ::oxplow_config::OxplowConfig,
                get_workspace_context => $crate::commands::config::get_workspace_context {} -> $crate::commands::config::WorkspaceContext,
                // lsp
                list_installed_lsp_packages => $crate::commands::lsp::list_installed_lsp_packages {} -> Vec<$crate::commands::lsp::InstalledLspPackage>,
                lsp_request => $crate::commands::lsp::lsp_request { stream_id: String, language_id: String, method: String, params_json: String } -> String,
                lsp_notify => $crate::commands::lsp::lsp_notify { stream_id: String, language_id: String, method: String, params_json: String } -> (),
                list_lsp_servers => $crate::commands::lsp::list_lsp_servers {} -> Vec<::oxplow_app::lsp_sessions::LspServerListing>,
                respond_lsp_apply_edit => $crate::commands::lsp::respond_lsp_apply_edit { token: u32, applied: bool, failure_reason: Option<String> } -> (),
                // terminal (open_terminal_session needs PluginRuntimeState; keystrokes aren't
                // a command: they ride the daemon's /events socket, `terminal_input`)
                close_terminal_session => $crate::commands::terminal::close_terminal_session { session_id: String } -> (),
                terminal_session_cwd => $crate::commands::terminal::terminal_session_cwd { session_id: String } -> Option<String>,
                terminate_terminal_session => $crate::commands::terminal::terminate_terminal_session { session_id: String } -> (),
                lookup_terminal_session => $crate::commands::terminal::lookup_terminal_session { uri: String } -> Option<String>,
                // snapshot
                list_snapshot_ops => $crate::commands::snapshot::list_snapshot_ops { stream_id: ::oxplow_domain::StreamId, limit: Option<u32> } -> Vec<::oxplow_db::SnapshotOp>,
                get_snapshot_stats => $crate::commands::snapshot::get_snapshot_stats { snapshot_id: ::oxplow_domain::SnapshotId } -> ::oxplow_db::SnapshotStats,
                get_blob_storage_bytes => $crate::commands::snapshot::get_blob_storage_bytes {} -> ::oxplow_domain::ByteSize,
                list_wiki_slugs_for_snapshots => $crate::commands::snapshot::list_wiki_slugs_for_snapshots { snapshot_ids: Vec<::oxplow_domain::SnapshotId> } -> Vec<(::oxplow_domain::SnapshotId, String)>,
                list_files_for_snapshot => $crate::commands::snapshot::list_files_for_snapshot { snapshot_id: ::oxplow_domain::SnapshotId } -> Vec<::oxplow_db::FileSnapshot>,
                get_file_snapshot => $crate::commands::snapshot::get_file_snapshot { file_snapshot_id: ::oxplow_domain::FileSnapshotId } -> Option<::oxplow_db::FileSnapshot>,
                // background
                list_background_tasks => $crate::commands::background::list_background_tasks {} -> Vec<::oxplow_app::BackgroundTask>,
                get_background_task => $crate::commands::background::get_background_task { id: String } -> Option<::oxplow_app::BackgroundTask>,
                start_background_task => $crate::commands::background::start_background_task { kind: ::oxplow_app::BackgroundTaskKind, label: String, detail: Option<String> } -> ::oxplow_app::BackgroundTask,
                complete_background_task => $crate::commands::background::complete_background_task { id: String, result_json: Option<String> } -> (),
                fail_background_task => $crate::commands::background::fail_background_task { id: String, error: String } -> (),
                update_background_task => $crate::commands::background::update_background_task { id: String, label: Option<String>, detail: Option<Option<String>>, progress: Option<Option<f64>> } -> (),
                // log

                // streams
                list_streams => $crate::commands::streams::list_streams {} -> ::std::vec::Vec<::oxplow_domain::Stream>,
                get_primary_stream => $crate::commands::streams::get_primary_stream {} -> Option<::oxplow_domain::Stream>,
                get_current_stream => $crate::commands::streams::get_current_stream {} -> Option<::oxplow_domain::Stream>,
                switch_stream => $crate::commands::streams::switch_stream { id: Option<::oxplow_domain::StreamId> } -> (),
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

    use oxplow_app::Services;

    /// Build an [`crate::RpcContext`] over an in-memory DB rooted at a
    /// fresh git repo (ensure_primary refuses non-git dirs). Returns a
    /// context with no plugin runtime — agent-spawn tests exercise the
    /// graceful-degradation path. Derefs to `Services`, so existing
    /// `svc.<store>` test usage keeps working. Keep the returned
    /// `TempDir` alive for the duration of the test.
    #[allow(clippy::unwrap_used)]
    pub async fn services() -> (crate::RpcContext, tempfile::TempDir) {
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
            services: Services::in_memory(dir.path()).await.unwrap(),
            plugin_runtime: None,
        };
        (ctx, dir)
    }

    /// Publish the generation the files written and the config changed since
    /// ask for, as the owner of generations does on the change.
    pub async fn publish(ctx: &crate::RpcContext) {
        // Only deployed extensions run: deploy the sources the test wrote
        // (but not ones the person turned off; one that doesn't load is
        // left out, as an activation holds it back).
        let root = ctx.services.layout.project_dir.clone();
        let off = oxplow_config::extension_settings(&root).personal.disable;
        let mut names: Vec<String> = std::fs::read_dir(root.join(".oxplow/extensions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !off.contains(n))
            .collect();
        names.sort();
        for name in names {
            let _ =
                oxplow_app::extension::extension_deploy::deploy(&ctx.services, &name, true).await;
        }
        ctx.services
            .extension_activation
            .activate(ctx.services.clone())
            .await
            .expect("the extensions activate");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn ping_routes_with_no_args() {
        let (svc, _dir) = services().await;
        let out = dispatch("ping", json!(null), &svc).await.unwrap();
        assert_eq!(out, json!("pong"));
    }

    #[tokio::test]
    async fn list_streams_routes_and_serializes() {
        let (svc, _dir) = services().await;
        // Empty object body works the same as a null body for no-arg cmds.
        let out = dispatch("list_streams", json!({}), &svc).await.unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    /// P6.E1b: the UI reads tasks through the datasets and writes them with
    /// `work_item.*` commands; the typed task RPCs are gone.
    #[tokio::test]
    async fn the_typed_task_rpcs_are_gone() {
        let (svc, _dir) = services().await;
        for name in [
            "get_thread_work_state",
            "get_backlog_state",
            "list_backlog",
            "get_task",
            "upsert_task",
            "create_task",
            "update_task",
            "delete_task",
            "reorder_tasks",
            "move_task",
            "list_work_item_efforts",
            "list_recently_finished",
            "clear_recently_finished",
        ] {
            let err = dispatch(name, json!({}), &svc).await.unwrap_err();
            assert_eq!(err.code, "NOT_FOUND", "{name}");
        }
    }

    /// P6.E2: the UI reads knowledge pages through the datasets.
    #[tokio::test]
    async fn the_wiki_read_rpcs_are_gone() {
        let (svc, _dir) = services().await;
        for name in [
            "list_wiki_pages",
            "search_wiki_titles",
            "read_wiki_page_body",
            "list_wiki_freshness",
        ] {
            let err = dispatch(name, json!({}), &svc).await.unwrap_err();
            assert_eq!(err.code, "NOT_FOUND", "{name}");
        }
    }

    #[tokio::test]
    async fn unknown_command_is_not_found() {
        let (svc, _dir) = services().await;
        let err = dispatch("no_such_command", json!({}), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "NOT_FOUND");
    }

    #[tokio::test]
    async fn bad_args_are_rejected_as_invalid() {
        let (svc, _dir) = services().await;
        // `snapshotIds` is required; an empty object can't deserialize into Args.
        let err = dispatch("list_efforts_at_snapshots", json!({}), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
