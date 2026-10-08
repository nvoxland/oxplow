//! Application services / use-cases layer.
//!
//! Constructs the dependency graph: Database → store impls →
//! services. The Tauri command crate and the MCP crate both call into
//! this layer; they never reach into infrastructure crates directly.
//!
//! Held inside `Arc<Services>` and registered as Tauri state. Methods
//! on `Services` are the high-level "use cases" the IPC layer calls.

pub mod acp;
pub mod advisories;
pub mod agent_command;
pub mod agent_context;
pub mod agent_path;
pub mod agent_policy;
pub mod agent_prompt;
pub mod agent_stall_watch;
pub mod agent_status_derive;
pub mod ai_compute;
pub mod ai_service;
pub mod assets;
pub mod attribution;
pub mod background_task;
pub mod blob_store;
pub mod boot;
pub mod branch_reconciler;
pub mod bundled_extensions;
pub mod capabilities;
pub mod change_analysis;
pub mod change_reactor;
pub mod churn;
pub mod client_host;
pub mod code_analysis;
pub mod code_intel;
pub mod code_intel_conformance;
pub mod code_quality_runner;
pub mod collection;
pub mod collector_runner;
pub mod collector_triggers;
pub mod commands;
pub mod commit_indexer;
pub mod commit_links;
pub mod component_bundles;
pub mod config_reactors;
pub mod config_service;
pub mod config_watch;
pub mod daemon_supervisor;
pub mod dashboard_tiles;
pub mod diagnostics;
pub mod duplication_scan;
pub mod effect_triggers;
pub mod effective_config;
pub mod effects;
pub mod effort_evidence;
pub mod effort_landing;
pub mod effort_lifecycle;
pub mod effort_observation;
pub mod effort_policy;
pub mod effort_reactors;
pub mod effort_service;
pub mod entity_metrics;
pub mod event_bodies;
pub mod event_lineage;
pub mod event_pump;
pub mod events;
pub mod exec_consent;
pub mod extension_catalog;
pub mod extension_commands;
pub mod extension_effects;
pub mod extension_event_types;
pub mod extension_models;
pub mod extension_ref_kinds;
pub mod extensions;
pub mod file_ref_version;
pub mod followup;
pub mod hook_ingest;
pub mod host_capabilities;
pub mod indexer;
pub mod inferred_decisions;
pub mod kind_search;
pub mod knowledge;
pub mod knowledge_conformance;
pub mod lens_actions;
pub mod lens_text;
pub mod link_check;
pub mod lsp_diagnostics;
#[cfg(test)]
mod lsp_fake;
pub mod lsp_installer;
pub mod lsp_sessions;
pub mod metric_bucket;
pub mod metric_cube;
pub mod metric_engine;
pub mod metric_findings;
pub mod metric_grid;
pub mod metric_visibility;
pub mod metrics_service;
pub mod models_changed;
pub mod net_sandbox;
pub mod otlp_ingest;
pub mod otlp_tokens;
pub mod output_activity;
pub mod pacing;
pub mod page_ref_backfill;
pub mod page_ref_consumers;
pub mod plugin_health;
pub mod plugin_repair;
pub mod post_tool_reactors;
pub mod producer_metrics;
pub mod prompt_catalog;
pub mod providers;
pub mod reasoning;
pub mod recovery;
pub mod ref_moves;
pub mod ref_resolver;
pub mod resume_check;
pub mod semantic_catalog;
pub mod snapshot_capture;
pub mod snapshot_capture_registry;
pub mod snapshot_conformance;
pub mod snapshot_content;
pub mod snapshot_files;
#[cfg(test)]
mod source_guards;
pub mod sql_gateway;
#[cfg(test)]
mod stream_service_tests;
pub mod symbol_collector;
pub mod terminal_sessions;
#[cfg(test)]
pub(crate) mod test_fixtures;
pub mod test_outcome;
pub mod test_signals;
pub mod thread_checkpoint;
pub mod thread_runtime;
pub mod token_usage;
pub mod tool_call_reactors;
pub mod tool_calls;
pub mod trees;
pub mod turn_snapshots;
pub mod ui_push;
pub mod vcs;
#[cfg(test)]
pub mod vcs_conformance;
pub mod vocabulary_reactor;
pub mod wiki_drift;
pub mod wiki_pages;
pub mod wiki_pages_watch;
pub mod work_item_reads;
pub mod work_items;
pub mod work_items_conformance;
pub mod workspace_files;
pub mod workspace_watch;
pub mod worktrees;
pub mod zones_service;

pub use agent_prompt::{
    build_session_context_block, build_session_context_block_with_role, role_change_banner,
    RoleMode,
};
pub use events::{event_channels, EventBus, OxplowEvent, WorkspaceChangeKind};
pub use hook_ingest::{
    HookEnvelope, HookIngestError, HookIngestService, IngestOutcome, ToolDecision,
};
pub use oxplow_lsp::{LspError, LspProxy};
use oxplow_tasks::{SqliteTaskLinkStore, SqliteTaskStore};

use oxplow_domain::vocabulary::VocabularyHandle;
use std::path::PathBuf;
use std::sync::Arc;

pub use background_task::{
    BackgroundTask, BackgroundTaskChange, BackgroundTaskChangeKind, BackgroundTaskKind,
    BackgroundTaskStatus, BackgroundTaskStore, StartInput, UpdateInput,
};
pub use followup::{Followup, FollowupStore};
// Re-export the effort store trait + row type so downstream crates that
// hold a `Services` (e.g. oxplow-control-plane, which doesn't depend on
// oxplow-db directly) can call the trait methods its public
// `effort_store` field exposes.
pub use oxplow_db::{EffortFile, EffortStore};

use thiserror::Error;
use tracing::info;

use std::sync::RwLock;

use oxplow_config::OxplowConfig;
use oxplow_db::{
    Database, SqliteAgentNudgeStore, SqliteAgentSessionStore, SqliteAgentTurnStore,
    SqliteCodeQualityStore, SqliteCommentStore, SqliteEffortStore, SqliteEventLogStore,
    SqliteFactStore, SqlitePageRefStore, SqlitePageVisitStore, SqliteSearchStore,
    SqliteSnapshotStore, SqliteStreamStore, SqliteThreadNoteStore, SqliteThreadStore,
    SqliteTokenUsageStore, SqliteUsageStore, SqliteWikiPageStore, SqliteWikiPageThreadUpdateStore,
};
use oxplow_domain::stores::AgentStatusStore;
use oxplow_session::{StreamService, ThreadService, WorkspaceLayout};

#[derive(Debug, Error)]
pub enum AppInitError {
    #[error("config: {0}")]
    Config(#[from] oxplow_config::ConfigError),
    #[error("db: {0}")]
    Db(#[from] oxplow_db::DbInitError),
    #[error("session: {0}")]
    Session(#[from] oxplow_session::SessionError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("capabilities: {0}")]
    Capabilities(String),
}

/// Layout of the on-disk state for one project. Lives under
/// `<project>/.oxplow/`.
pub struct AppLayout {
    pub project_dir: PathBuf,
    pub state_dir: PathBuf,
    pub state_db_path: PathBuf,
}

impl AppLayout {
    pub fn for_project(project_dir: impl Into<PathBuf>) -> Self {
        let project_dir = project_dir.into();
        let state_dir = project_dir.join(".oxplow");
        let state_db_path = state_dir.join("local.sqlite");
        Self {
            project_dir,
            state_dir,
            state_db_path,
        }
    }

    /// Path of the per-project single-instance lock file.
    pub fn instance_lock_path(&self) -> PathBuf {
        self.state_dir.join("instance.lock")
    }
}

/// Ensure the `.oxplow/` state directory exists and carries a `.gitignore`
/// that keeps the project config tracked but ignores oxplow's local state.
///
/// Creates `state_dir` (recursively) and drops a `.gitignore` that ignores
/// everything in the directory except `project.yaml` (the shareable config)
/// and the `.gitignore` itself — so a project can commit its oxplow config
/// while the DB / snapshots / wiki / runtime files stay local. Idempotent:
/// an existing `.gitignore` is left untouched. Whether the directory as a
/// whole is tracked is the host repo's call (its root `.gitignore`); oxplow
/// doesn't manage that.
pub fn ensure_state_dir(state_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let gitignore = state_dir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(gitignore, "*\n!.gitignore\n!project.yaml\n")?;
    }
    Ok(())
}

/// Try to take the per-project single-instance lock. On success the
/// held [`std::fs::File`] is returned — keep it alive for the whole
/// process (the OS releases the advisory lock when it drops). `None`
/// means another live oxplow process already holds it, so this process
/// must not boot a second `Services` on the same `local.sqlite`
/// (double fs/git watchers + a serialized SQLite writer lock).
pub fn try_acquire_instance_lock(layout: &AppLayout) -> std::io::Result<Option<std::fs::File>> {
    use fs2::FileExt;
    ensure_state_dir(&layout.state_dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(layout.instance_lock_path())?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(file)),
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e),
    }
}

/// Non-destructive probe: is `project_dir`'s instance lock held — i.e.
/// does it already have a live `oxplow-daemon`? Backs
/// [`wait_for_project_unlock`]. Acquiring the lock here would itself
/// succeed when nobody holds it, so we immediately drop it (releasing)
/// and report the prior state.
pub fn is_project_locked(project_dir: &std::path::Path) -> bool {
    use fs2::FileExt;
    let lock_path = project_dir.join(".oxplow").join("instance.lock");
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(&lock_path) else {
        return false; // no lock file → never opened (or not yet)
    };
    match file.try_lock_exclusive() {
        // We took it → nobody else held it. Drop releases immediately.
        Ok(()) => false,
        Err(_) => true,
    }
}

/// Wait for `project_dir`'s instance lock to come free, up to `timeout`.
/// Returns whether it did.
///
/// Used after killing an orphaned daemon: the signal returns long before
/// the process is gone, and starting a replacement while the old one
/// still holds the lock just makes the new daemon exit. Polls the lock
/// itself rather than the pid — the lock is the thing that has to be
/// free, and a zombie pid would still look alive.
pub fn wait_for_project_unlock(
    project_dir: &std::path::Path,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !is_project_locked(project_dir) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(test)]
mod instance_lock_tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_releases() {
        let dir = tempfile::tempdir().unwrap();
        let layout = AppLayout::for_project(dir.path());

        let held = try_acquire_instance_lock(&layout).unwrap();
        assert!(held.is_some(), "first acquire succeeds");
        assert!(
            is_project_locked(dir.path()),
            "probe sees the lock while held"
        );

        drop(held);
        assert!(
            !is_project_locked(dir.path()),
            "probe is clear once the lock is released"
        );
    }

    #[test]
    fn unopened_project_is_not_locked() {
        let dir = tempfile::tempdir().unwrap();
        // No .oxplow/instance.lock yet.
        assert!(!is_project_locked(dir.path()));
    }

    /// The common case after killing an orphaned daemon: nothing holds
    /// the lock, so the replacement can start at once.
    #[test]
    fn waiting_on_a_free_lock_returns_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        assert!(wait_for_project_unlock(
            dir.path(),
            std::time::Duration::from_secs(5)
        ));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "a free lock should not be waited on"
        );
    }

    /// A lock that never frees has to time out rather than hang the
    /// window that's trying to open.
    #[test]
    fn waiting_on_a_held_lock_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let layout = AppLayout::for_project(dir.path());
        let _held = try_acquire_instance_lock(&layout).unwrap().unwrap();

        assert!(!wait_for_project_unlock(
            dir.path(),
            std::time::Duration::from_millis(150)
        ));
    }

    /// The point of the wait: a lock released while we're waiting is
    /// picked up, not slept through.
    #[test]
    fn waiting_returns_as_soon_as_the_lock_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let layout = AppLayout::for_project(dir.path());
        let held = try_acquire_instance_lock(&layout).unwrap().unwrap();

        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(held);
        });
        assert!(wait_for_project_unlock(
            dir.path(),
            std::time::Duration::from_secs(5)
        ));
        releaser.join().unwrap();
    }

    #[test]
    fn ensure_state_dir_writes_gitignore_tracking_only_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".oxplow");

        ensure_state_dir(&state_dir).unwrap();

        assert!(state_dir.is_dir());
        let gitignore = std::fs::read_to_string(state_dir.join(".gitignore")).unwrap();
        // Ignore everything except the shareable config (and the gitignore).
        assert_eq!(gitignore, "*\n!.gitignore\n!project.yaml\n");
    }

    #[test]
    fn ensure_state_dir_is_idempotent_and_preserves_existing_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".oxplow");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join(".gitignore"), "custom\n").unwrap();

        ensure_state_dir(&state_dir).unwrap();

        let gitignore = std::fs::read_to_string(state_dir.join(".gitignore")).unwrap();
        assert_eq!(
            gitignore, "custom\n",
            "existing .gitignore is not clobbered"
        );
    }
}

/// What differs between the app and [`Services::in_memory`]: this
/// machine's keychain, global config dir and approvals (stand-ins in
/// tests), and how long a failed provider waits before restarting.
struct MachineEnv {
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    config_dir: Option<PathBuf>,
    /// Where program approvals live; `None` = this machine's oxplow home.
    approvals_file: Option<PathBuf>,
    /// A failed provider's first restart wait (doubling from there).
    provider_backoff: std::time::Duration,
    /// Where approved provider copies run from, outside the repo.
    provider_copies: PathBuf,
    /// How long a provider's `check` or `invoke` may take.
    provider_call_timeout: std::time::Duration,
    /// How long a code-intelligence request waits for its language server.
    lsp_request_timeout: std::time::Duration,
    /// The environment a provider's declared `env` is read from: this
    /// process's, or the one a test or `plugin test` run names.
    host_env: providers::host::HostEnv,
}

/// All the long-lived services oxplow needs to serve a UI.
///
/// Registered with Tauri as `tauri::State<Arc<Services>>`, so the
/// renderer never clones `Services` directly — every reader bumps the
/// `Arc` refcount instead. The inner pieces (PtyManager, EventBus,
/// SqliteSnapshotStore, etc.) all derive `Clone` and route through
/// shared owner tasks via `mpsc`/`broadcast`, so even an accidental
/// `Services.clone()` doesn't spawn a duplicate runtime — it just
/// hands out another sender into the same backing task.
pub struct Services {
    pub config: Arc<RwLock<OxplowConfig>>,
    /// Woken each time the in-memory config is swapped (`oxplow.config.set`'s
    /// after-commit apply) — what a `config.changed` reactor waits on.
    pub config_applied: Arc<tokio::sync::Notify>,
    pub db: Database,
    pub layout: AppLayout,
    pub streams: StreamService,
    pub threads: ThreadService,
    /// What an effort's lifecycle does around its store (snapshot pins,
    /// lifecycle metrics, file claims).
    pub efforts: effort_service::EffortService,
    pub stream_store: Arc<SqliteStreamStore>,
    pub thread_store: Arc<SqliteThreadStore>,
    /// A thread's agent sessions (`agent_session`).
    pub agent_session_store: Arc<SqliteAgentSessionStore>,
    pub task_store: Arc<SqliteTaskStore>,
    pub thread_note_store: Arc<SqliteThreadNoteStore>,
    pub task_link_store: Arc<SqliteTaskLinkStore>,
    /// The event log (`.context/data-model.md` "event_log"). Producers
    /// append inside their own transaction via `event_log_store::append_tx`;
    /// this handle is for reads and the pump.
    pub event_log_store: Arc<SqliteEventLogStore>,
    /// The one way a query reaches the semantic layer (P4.1).
    pub sql: sql_gateway::SqlGateway,
    /// When each model last changed (P4.6); `models_changed::spawn` keeps
    /// it and announces `ModelsChanged`.
    pub model_watermarks: Arc<models_changed::ModelWatermarks>,
    /// The assets (P7.B1): derived data recomputed when its input tables
    /// change — the metric cube; `models_changed::spawn` tells them.
    pub assets: assets::Assets,
    /// Every event `type@v` the log accepts, with its schema. Core types
    /// at boot; plugin types join when their manifests load.
    pub vocabulary: VocabularyHandle,
    /// Delivers the log to its consumers (checkpoints, dead letters).
    /// Producers `wake()` it after they commit; `boot.rs` spawns the loop.
    pub event_pump: Arc<event_pump::EventPump>,
    /// Loaded extensions per worktree root, reloaded when a file under
    /// `oxplow/extensions/` or the project config changes.
    pub extension_catalog: Arc<extension_catalog::ExtensionCatalog>,
    /// Custom components' bundles as their frames loaded them, by version
    /// (tsk984): what the daemon serves a frame, and what its invoke names.
    pub component_bundles: Arc<component_bundles::ComponentBundles>,
    /// The primary worktree's extensions' SQL models, and their errors (P4.9).
    pub extension_models: Arc<extension_models::ExtensionModelsService>,
    /// Rebuilds `vocabulary` from the primary worktree's extensions, and
    /// their refused declarations (P8.D3).
    pub vocabulary_service: Arc<vocabulary_reactor::VocabularyService>,
    /// The command bus: the one write path (`.context/commands.md`).
    pub commands: Arc<commands::CommandBus>,
    /// The work-items providers, by name (`.context/work-items.md`).
    pub work_items: oxplow_domain::work_items::WorkItemsRegistry,
    /// Every capability's implementations and which is active.
    pub capabilities: Arc<capabilities::CapabilityRegistry>,
    /// The enabled external provider instances (`.context/providers.md`).
    pub providers: Arc<providers::ProviderRegistry>,
    /// Enabled extensions' `commands:` on the bus (P6b).
    pub extension_commands: Arc<extension_commands::ExtensionCommands>,
    /// Filled at boot (`effect_triggers::register`): the services the
    /// effect operations run against.
    pub effect_services: commands::effect::ServicesSlot,
    /// The daemon's line to its window: the window's capabilities' calls.
    pub client_host: Arc<client_host::ClientHost>,
    /// The knowledge provider: oxplow's wiki (`.context/knowledge.md`).
    pub knowledge: Arc<dyn oxplow_domain::knowledge::KnowledgeProvider>,
    pub wiki_page_store: Arc<SqliteWikiPageStore>,
    pub page_visit_store: Arc<SqlitePageVisitStore>,
    pub usage_store: Arc<SqliteUsageStore>,
    pub code_quality_store: Arc<SqliteCodeQualityStore>,
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    /// Unified site-wide search index (FTS5/BM25). Written by the search
    /// kinds' assets (`kind_search.rs`) and, for files, the `search.index`
    /// consumer (`indexer.rs`); read by the `search` IPC/MCP surface.
    pub search_store: Arc<SqliteSearchStore>,
    /// Per-stream snapshot capture registry. Holds one service per
    /// active stream (each watching its own worktree). Callers that know which stream they're acting on `get(&stream_id)`
    /// here; those about the primary stream's worktree (the wiki watcher)
    /// use `snapshot_captures.primary()`.
    pub snapshot_captures: snapshot_capture_registry::SnapshotCaptureRegistry,
    pub agent_status_store: Arc<dyn AgentStatusStore>,
    pub agent_turn_store: Arc<SqliteAgentTurnStore>,
    /// Backing in-memory state for hook events + agent status. Both
    /// `hook_event_store` and `agent_status_store` are trait-object
    /// views of this same registry — keep the concrete handle around
    /// for code that wants to bypass the trait surfaces.
    pub thread_runtime: Arc<thread_runtime::ThreadRuntimeRegistry>,
    pub effort_store: Arc<SqliteEffortStore>,
    /// Durable atomic fact layer (epic tsk12) — the unified metric substrate
    /// (the V38 `metric_*` cluster is retired, T-E3). Producers write facts
    /// here; the read surface aggregates them through `metric_engine`.
    pub fact_store: Arc<SqliteFactStore>,
    /// Aggregation engine over `fact_store` (metrics-as-specs; epic tsk12).
    pub metric_engine: metric_engine::MetricEngine,
    /// The metric-ancestry resolver (tsk102) — ONE instance shared by the
    /// engine's fact fold and the cube builder's seed, so the two can never
    /// disagree on what a branch sees.
    pub metric_visibility: Arc<metric_visibility::VisibilityResolver>,
    /// Runs config-declared `metrics:` gauges into the substrate (tsk213, P3):
    /// seeds definitions, runs on-snapshot/on-effort-complete/manual triggers.
    pub metrics: metrics_service::MetricsService,
    /// Persisted agent nudges (report-less-run / coverage-target) — the
    /// human-facing record of what oxplow steered the agent to do.
    pub nudge_store: Arc<SqliteAgentNudgeStore>,
    /// User-created dashboards (grids of metric tiles) — project-global (tsk138).
    pub dashboard_store: Arc<oxplow_db::SqliteDashboardStore>,
    /// Extension-source entity data + run state (see `collector_runner`).
    pub collector_store: Arc<oxplow_db::SqliteCollectorStore>,
    /// Runs collectors (the `oxplow.collector.sync` command and the scheduler).
    pub collector_runner: collector_runner::CollectorRunner,
    /// An agent's answers in threads (`v_thread_answer`, `oxplow.lens.show`).
    pub thread_answer_store: oxplow_db::SqliteThreadAnswerStore,
    /// The person's left-nav layout (P6.G1).
    pub panel_layout_store: oxplow_db::SqlitePanelLayoutStore,
    /// Agent decisions and claims (`v_decision`, `v_claim`).
    pub reasoning_store: Arc<oxplow_db::SqliteReasoningStore>,
    /// Persisted agent tool calls (`v_tool_call` and derived views).
    pub tool_call_store: Arc<oxplow_db::SqliteToolCallStore>,
    /// Git history and branches (`v_commit`, `v_branch`, …).
    pub git_store: Arc<oxplow_db::SqliteGitStore>,
    pub diagnostic_store: Arc<oxplow_db::SqliteDiagnosticStore>,
    /// The symbol index (`v_symbol`), written by the symbol collector.
    pub symbol_store: oxplow_db::SqliteSymbolStore,
    /// Oxplow's own model calls by role (`v_ai_call` records each one).
    pub ai: Arc<ai_service::AiService>,
    /// Recorded AI computations over `ai` (`ai_result`).
    pub ai_compute: Arc<ai_compute::AiCompute>,
    /// Stored change analysis (`v_change*`) and its producer state.
    pub change_store: Arc<oxplow_db::SqliteChangeStore>,
    pub change_analyzer: Arc<change_analysis::ChangeAnalyzer>,
    /// Runs extension advisories for the hooks (see `advisories`).
    pub advisories: Arc<advisories::AdvisoryRunner>,
    /// Per-effort metric deltas / observations for lenses (see `effort_evidence`).
    pub effort_evidence_store: Arc<oxplow_db::SqliteEffortEvidenceStore>,
    /// Secrets store (the OS keychain in the app): AI provider keys and
    /// extension-source credentials. Values never go to the UI or agents.
    pub secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    /// Collection engine (passive Bash-hook detection + coverage ingest).
    pub collection: collection::CollectionService,
    /// Per-turn agent token usage parsed from the hook transcript (tsk104).
    pub token_usage_store: Arc<SqliteTokenUsageStore>,
    /// Captures token usage on Stop from the agent transcript.
    pub token_usage: token_usage::TokenUsageService,
    /// Logs an agent's OTLP token exports as `agent.tokens.reported`.
    pub otlp_ingest: otlp_ingest::OtlpIngestService,
    pub wiki_page_thread_updates: Arc<SqliteWikiPageThreadUpdateStore>,
    /// Unified cross-page reference graph. Every writer that owns a
    /// `source_kind` slice mirrors its outbound refs into this store
    /// at write time; the reader IPC (`list_backlinks` /
    /// `list_outbound`) exposes the inverse view.
    pub page_ref_store: Arc<SqlitePageRefStore>,
    pub comment_store: Arc<SqliteCommentStore>,
    pub hook_ingest: HookIngestService,
    pub background_tasks: BackgroundTaskStore,
    pub followups: FollowupStore,
    pub pty: oxplow_pty::PtyManager,
    pub blobs: blob_store::BlobStore,
    /// Reads a captured file's bytes from whichever store holds them.
    pub snapshot_content: snapshot_content::SnapshotContent,
    pub lsp_sessions: lsp_sessions::LspSessionManager,
    /// Code intelligence from the language servers (`.context/lsp.md`).
    pub code_intel: Arc<dyn oxplow_domain::code_intel::CodeIntelligence>,
    /// The write guard, shared by every agent transport (the hook route,
    /// ACP).
    pub agent_policy: Arc<agent_policy::AgentPolicy>,
    /// Recording and prompt context shared by every agent transport.
    pub agent_context: Arc<agent_context::AgentContext>,
    /// Open ACP agent sessions (tsk281). Sessions get a
    /// `acp::host::ServicesAcpHost` holding `Services` weakly.
    pub acp: Arc<acp::manager::AcpManager>,
    /// This machine's approvals of the project's programs (`exec_consent`).
    pub approvals: Arc<exec_consent::ApprovalStore>,
    pub lsp_installer: lsp_installer::LspInstallerService,
    pub terminal_sessions: terminal_sessions::TerminalSessionRegistry,
    /// Shared per-thread PTY liveness, written by the terminal forwarder
    /// and read by the agent stall watchdog (tsk141).
    pub output_activity: output_activity::OutputActivity,
    pub recovery: recovery::RecoveryService,
    pub events: EventBus,
    /// Git's own operations beyond the capability (rebase, ignore,
    /// change scopes, ref labels, …); the same provider as `vcs`.
    pub git: vcs::GitProvider,
    /// The VCS capability (`.context/vcs.md`): git, as a provider.
    pub vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
    /// Which directory each stream works in.
    pub worktrees: Arc<worktrees::WorktreeRouter>,
    /// Every version of a workspace's tree — working, snapshot, VCS
    /// revision — read and diffed through one interface.
    pub trees: Arc<trees::Trees>,
    /// A stream's files: list, read, write.
    pub workspace_files: Arc<workspace_files::WorkspaceFiles>,
    /// Keeps `stream.branch` equal to the checked-out branch; spawned at
    /// boot.
    pub branch_reconciler: Arc<branch_reconciler::BranchReconciler>,
    /// A stream's refs moved (P7.B6): what the backend's ref listeners
    /// subscribe to instead of the event bus.
    pub ref_moves: ref_moves::RefMoves,
}

impl Services {
    /// The extensions under `root` as listed: loaded, with what the
    /// primary worktree's model compile and vocabulary refused added to
    /// each one's errors.
    pub async fn listed_extensions(&self, root: &std::path::Path) -> Vec<extensions::Extension> {
        let loaded = self.extension_catalog.get(root).to_vec();
        let loaded = self.extension_models.with_health(root, loaded).await;
        self.vocabulary_service.with_health(root, loaded).await
    }

    /// Reading and restoring captured files (`snapshot_files`).
    pub fn snapshot_files(&self) -> snapshot_files::SnapshotFiles {
        snapshot_files::SnapshotFiles {
            snapshots: self.snapshot_store.clone(),
            streams: self.stream_store.clone(),
            content: self.snapshot_content.clone(),
            project_dir: self.layout.project_dir.clone(),
        }
    }

    /// The `work_item.*` commands, typed (`work_items::WorkItems`): the
    /// one write surface for every provider's items.
    pub fn work_items_client(&self) -> work_items::WorkItems {
        work_items::WorkItems::new(self.commands.clone())
    }

    /// The tree a thread works in: its stream's worktree (a sibling
    /// directory for a worktree stream), else the project dir. What its
    /// tool paths are relative to (tsk350 policy, tsk386 claims).
    /// What running a thread's advisories needs.
    pub fn advisory_deps(&self) -> advisories::AdvisoryDeps {
        advisories::AdvisoryDeps {
            advisories: self.advisories.clone(),
            effort_store: self.effort_store.clone(),
            thread_store: self.thread_store.clone(),
            worktrees: self.worktrees.clone(),
            approvals: self.approvals.clone(),
            extension_catalog: self.extension_catalog.clone(),
            db: self.db.clone(),
            sql: self.sql.clone(),
            collection: self.collection.clone(),
            capabilities: self.capabilities.clone(),
            config: self.config.clone(),
        }
    }

    pub async fn thread_worktree(&self, thread: &oxplow_domain::ThreadId) -> PathBuf {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let project = self.layout.project_dir.clone();
        let Some(t) = self.thread_store.get(thread).await.ok().flatten() else {
            return project;
        };
        self.stream_store
            .list()
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.id == t.stream_id)
            .map(|s| PathBuf::from(s.worktree_path))
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(project)
    }

    /// Bootstrap. Run once at app startup. `secrets` is where keys,
    /// tokens and the approval key live: the OS keychain for the shipped
    /// daemon, always; memory only for the browser suite's
    /// `oxplow-daemon-sim` (tsk948).
    pub fn boot(
        layout: AppLayout,
        secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    ) -> Result<Self, AppInitError> {
        ensure_state_dir(&layout.state_dir)?;

        let config = oxplow_config::load_project_config(&layout.project_dir)?;
        info!(project = %layout.project_dir.display(), agents = ?config.agents, "config loaded");

        let db = Database::open(&layout.state_db_path)?;
        let machine = MachineEnv {
            secrets,
            config_dir: oxplow_config::global_config_dir(),
            approvals_file: None,
            provider_backoff: std::time::Duration::from_secs(1),
            provider_copies: oxplow_config::global_config_dir()
                .unwrap_or_else(|| layout.state_dir.join("global-config"))
                .join("provider-copies")
                .join(collector_runner::project_key(&layout.project_dir)),
            provider_call_timeout: std::time::Duration::from_secs(60),
            lsp_request_timeout: std::time::Duration::from_secs(30),
            host_env: providers::host::process_env(),
        };
        Self::build(layout, config, db, machine)
    }

    /// Shared construction core for [`Self::boot`] and
    /// [`Self::in_memory`]: every store, service, and registry is
    /// wired here, in dependency order, exactly once. The two
    /// entrypoints differ only in how they resolve the layout,
    /// config, and `Database` handle.
    fn build(
        layout: AppLayout,
        config: OxplowConfig,
        db: Database,
        machine: MachineEnv,
    ) -> Result<Self, AppInitError> {
        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
        let agent_session_store = Arc::new(SqliteAgentSessionStore::new(db.clone()));
        let page_ref_store = Arc::new(SqlitePageRefStore::new(db.clone()));
        let vocabulary = VocabularyHandle::core();
        let comment_store = Arc::new(SqliteCommentStore::new(db.clone(), vocabulary.clone()));
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let thread_note_store = Arc::new(SqliteThreadNoteStore::new(db.clone()));
        let task_link_store = Arc::new(SqliteTaskLinkStore::new(db.clone()));
        let event_log_store = Arc::new(SqliteEventLogStore::new(db.clone(), vocabulary.clone()));
        let sql = sql_gateway::SqlGateway::new(db.clone());
        let event_bus = EventBus::new();
        let ref_moves = ref_moves::RefMoves::new(event_bus.clone());
        let event_pump = Arc::new(event_pump::EventPump::new(
            db.clone(),
            (*event_log_store).clone(),
            vec![
                // A list's records reach the interface before an item's
                // page refs are restated from it.
                Arc::new(work_items::WorkItemsProjection),
                Arc::new(page_ref_consumers::PageRefWorkItemConsumer {
                    vocabulary: vocabulary.clone(),
                }),
                Arc::new(tool_call_reactors::ToolCallProjection),
                Arc::new(knowledge::WikiAttribution),
                // The log's facts the renderer hears (P7.B6).
                Arc::new(ui_push::UiPush {
                    events: event_bus.clone(),
                }),
            ],
        ));
        let wiki_page_store = Arc::new(SqliteWikiPageStore::new(db.clone()));
        let page_visit_store = Arc::new(SqlitePageVisitStore::new(db.clone()));
        let usage_store = Arc::new(SqliteUsageStore::new(db.clone()));
        let code_quality_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        // Git-backed snapshot rows are hashed lazily into the one content
        // identity space (xxh3) the first time a comparison needs them
        // (`.context/data-model.md` "snapshot + file_snapshot"). Every
        // worktree shares the repo's object db, so the primary dir serves
        // any stream's OIDs.
        let vcs: Arc<dyn oxplow_domain::vcs::Vcs> = Arc::new(vcs::GitProvider);
        let blobs = blob_store::BlobStore::new(layout.state_dir.join("snapshots"));
        let snapshot_content = snapshot_content::SnapshotContent::new(
            blobs.clone(),
            vcs.object_store(&layout.project_dir),
        );
        let snapshot_store = Arc::new({
            let content = snapshot_content.clone();
            SqliteSnapshotStore::with_vocabulary(db.clone(), vocabulary.clone())
                .with_content_hasher(Arc::new(move |oid: &str| content.object_content_hash(oid)))
        });
        let search_store = Arc::new(SqliteSearchStore::new(db.clone()));
        let thread_runtime = Arc::new(thread_runtime::ThreadRuntimeRegistry::new());
        let agent_status_store: Arc<dyn AgentStatusStore> = Arc::new(
            oxplow_db::SqliteAgentStatusStore::new(db.clone(), vocabulary.clone()),
        );
        let agent_turn_store = Arc::new(SqliteAgentTurnStore::with_vocabulary(
            db.clone(),
            vocabulary.clone(),
        ));
        let effort_store = Arc::new(SqliteEffortStore::with_vocabulary(
            db.clone(),
            vocabulary.clone(),
        ));
        let fact_store = Arc::new(SqliteFactStore::new(db.clone()));
        let metric_visibility = Arc::new(metric_visibility::VisibilityResolver::new(
            SqliteSnapshotStore::new(db.clone()),
            vcs.revision_graph(&layout.project_dir),
        ));
        let metric_engine = metric_engine::MetricEngine::new(SqliteFactStore::new(db.clone()))
            .with_visibility(metric_visibility.clone());
        let model_watermarks = Arc::new(models_changed::ModelWatermarks::default());
        let assets = assets::Assets::new(db.clone(), assets::COALESCE);
        let sql = sql
            .with_engine(metric_engine.clone())
            .with_watermarks(model_watermarks.clone());
        let nudge_store = Arc::new(SqliteAgentNudgeStore::new(db.clone()));
        let dashboard_store = Arc::new(oxplow_db::SqliteDashboardStore::new(db.clone()));
        let collector_store = Arc::new(oxplow_db::SqliteCollectorStore::new(db.clone()));
        let reasoning_store = Arc::new(oxplow_db::SqliteReasoningStore::new(db.clone()));
        let tool_call_store = Arc::new(oxplow_db::SqliteToolCallStore::new(db.clone()));
        let git_store = Arc::new(oxplow_db::SqliteGitStore::new(db.clone()));
        let diagnostic_store = Arc::new(oxplow_db::SqliteDiagnosticStore::new(db.clone()));
        let symbol_store = oxplow_db::SqliteSymbolStore::new(db.clone());
        let effort_evidence_store = Arc::new(oxplow_db::SqliteEffortEvidenceStore::new(db.clone()));
        let change_store = Arc::new(oxplow_db::SqliteChangeStore::new(db.clone()));
        let wiki_page_thread_updates = Arc::new(SqliteWikiPageThreadUpdateStore::new(db.clone()));

        let workspace_layout = WorkspaceLayout::for_project(&layout.project_dir);
        let config_arc = Arc::new(RwLock::new(config));
        // A stream's seeded thread runs the project's default agent, as
        // the config says it when the thread is made (tsk970).
        let streams = StreamService::new(
            workspace_layout,
            vcs.clone(),
            stream_store.clone(),
            thread_store.clone(),
            agent_session_store.clone(),
            {
                let config = config_arc.clone();
                Arc::new(move || {
                    oxplow_config::default_thread_agent(&config_service::read_config(&config))
                })
            },
        );
        let threads = ThreadService::new(thread_store.clone());

        let hook_ingest = HookIngestService::new(
            db.clone(),
            vocabulary.clone(),
            layout.project_dir.clone(),
            event_bus.clone(),
        )
        .with_event_pump(event_pump.clone());
        let recovery_svc = recovery::RecoveryService::new(agent_turn_store.clone());

        let pty = oxplow_pty::PtyManager::spawn();
        // Lazily-built per-(stream, language) LSP proxies. Spawn cost
        // is paid on first request, not at boot.
        let project_config = config_arc.clone();
        // Program approvals: this machine's, outside the repo (tsk344).
        let approvals = Arc::new(match machine.approvals_file.clone() {
            Some(f) => {
                exec_consent::ApprovalStore::at(f, &layout.project_dir, machine.secrets.clone())
            }
            None => exec_consent::ApprovalStore::for_project(
                &layout.project_dir,
                machine.secrets.clone(),
            ),
        });
        let ai = Arc::new(
            ai_service::AiService::new(
                oxplow_ai::client::Client::default(),
                machine.secrets.clone(),
                Arc::new(oxplow_db::SqliteAiCallStore::new(db.clone())),
                machine.config_dir.clone(),
            )
            // Read live, so project.yaml edits and reloads apply at once.
            .with_project_overrides(Arc::new(move || {
                ai_service::project_overrides(
                    &project_config
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .ai_roles,
                )
            })),
        );
        let ai_compute = Arc::new(ai_compute::AiCompute::new(
            ai.clone(),
            oxplow_db::SqliteAiResultStore::new(db.clone()),
        ));
        let lsp = lsp_sessions::LspSessionManager::new(config_arc.clone());
        let lsp_installer_svc =
            lsp_installer::LspInstallerService::new(&layout.state_dir, lsp.clone());
        if let Err(e) = futures::executor::block_on(lsp_installer_svc.replay_into_sessions()) {
            tracing::warn!(?e, "lsp installer manifest replay failed");
        }
        // Shared PTY liveness — the terminal forwarder stamps it, the
        // stall watchdog reads it (tsk141).
        let output_activity = output_activity::OutputActivity::new();
        let terminal_sessions =
            terminal_sessions::TerminalSessionRegistry::new(pty.clone(), output_activity.clone());
        let worktrees = Arc::new(worktrees::WorktreeRouter::new(
            layout.project_dir.clone(),
            stream_store.clone(),
        ));
        let code_intel: Arc<dyn oxplow_domain::code_intel::CodeIntelligence> =
            Arc::new(code_intel::LspProvider::new(
                lsp.clone(),
                worktrees.clone(),
                (*diagnostic_store).clone(),
                machine.lsp_request_timeout,
            ));
        let workspace_files = Arc::new(workspace_files::WorkspaceFiles::new(
            worktrees.clone(),
            vcs.clone(),
            event_bus.clone(),
        ));
        let branch_reconciler = Arc::new(branch_reconciler::BranchReconciler::new(
            worktrees.clone(),
            vcs.clone(),
            stream_store.clone(),
            ref_moves.clone(),
        ));
        let git = vcs::GitProvider;
        let trees = Arc::new(trees::Trees::new(
            vcs.clone(),
            snapshot_store.clone(),
            blobs.clone(),
            config_arc.clone(),
        ));

        // Snapshot capture singleton — owned here so anything in
        // Services can request snapshots. The fs-watcher, startup sweep, and
        // cleanup loop are spawned by the host binary (main.rs).
        let (max_bytes, workspace_filter) = {
            let g = config_arc.read();
            let max_bytes = g
                .as_ref()
                .map(|c| c.snapshot_max_file_bytes)
                .unwrap_or(5 * 1024 * 1024);
            let filter = g
                .as_ref()
                .map(|c| {
                    oxplow_fs_watch::WorkspaceFilter::for_project(
                        &layout.project_dir,
                        &c.generated.exclude,
                        &c.generated.include,
                    )
                })
                .unwrap_or_default();
            (max_bytes, filter)
        };
        // Snapshot capture is per-stream: each worktree gets its own
        // service so fs-watch sees edits in the right tree. The
        // registry holds them all and is the lookup point for any
        // code that knows the stream it's acting on.
        let primary_stream = futures::executor::block_on(streams.ensure_primary())?;
        let snapshot_captures = snapshot_capture_registry::SnapshotCaptureRegistry::new(
            snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                vcs: vcs.clone(),
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: max_bytes,
                workspace_filter,
                open_turn_probe: Some({
                    let turns = agent_turn_store.clone();
                    Arc::new(move |stream| {
                        let turns = turns.clone();
                        Box::pin(async move {
                            // On a lookup error, assume a turn is open: the
                            // turn's own end take is the safe default.
                            turns.stream_has_open_turn(stream).await.unwrap_or(true)
                        })
                    })
                }),
            },
        );
        // Register every active stream. Streams whose worktree no
        // longer exists on disk (orphaned) are silently skipped — the
        // registry's `register` returns None for those.
        let active_streams = futures::executor::block_on(streams.list_streams())?;
        for s in &active_streams {
            snapshot_captures.register(s);
        }
        snapshot_captures.set_primary(primary_stream.id);
        let hook_ingest =
            hook_ingest.with_turn_snapshots(Arc::new(turn_snapshots::CaptureTurnSnapshots {
                captures: snapshot_captures.clone(),
                threads: thread_store.clone(),
                efforts: effort_store.clone(),
                config: config_arc.clone(),
            }));
        // An agent's PTY exiting ends its session (Codex posts no SessionEnd).
        terminal_sessions.ingest_exits_into(hook_ingest.clone());
        // Built before the metric runner, which reports whole-tree collector sweeps
        // through it (tsk48).
        let background_tasks = BackgroundTaskStore::new();
        bridge_background_task_events(&background_tasks, &event_bus);
        let followups = FollowupStore::new();
        bridge_followup_events(&followups, &event_bus);

        // The metric runner (fact collectors → substrate). Holds leaf Arcs
        // only (never `Arc<Services>`); the `collector.triggers` consumer
        // hands it events, and its catalog loop is spawned in `boot.rs`.
        let extension_catalog = Arc::new(extension_catalog::ExtensionCatalog::for_project(
            &layout.project_dir,
        ));
        let extension_models = Arc::new(extension_models::ExtensionModelsService::new(
            db.clone(),
            extension_catalog.clone(),
            layout.project_dir.clone(),
        ));
        let metrics = metrics_service::MetricsService::new(
            snapshot_store.clone(),
            thread_store.clone(),
            effort_store.clone(),
            snapshot_content.clone(),
            vcs.clone(),
            config_arc.clone(),
            layout.project_dir.clone(),
        )
        .with_fact_store(fact_store.clone())
        .with_background_tasks(background_tasks.clone())
        .with_approvals(approvals.clone())
        .with_extension_catalog(extension_catalog.clone())
        .with_snapshot_captures(snapshot_captures.clone())
        .with_run_log(collector_runner::RunLog {
            db: db.clone(),
            vocabulary: event_log_store.vocabulary().clone(),
            layer: sql.clone(),
        });
        let efforts =
            effort_service::EffortService::new(effort_store.clone(), thread_store.clone())
                .with_event_pump(event_pump.clone())
                .with_snapshot_captures(snapshot_captures.clone())
                .with_metrics(fact_store.clone(), event_bus.clone())
                .with_steering_sources(agent_turn_store.clone(), comment_store.clone());
        // The post-commit half of effort open/close runs on the pump.
        event_pump.register_async(Arc::new(effort_lifecycle::EffortLifecycleConsumer::new(
            efforts.without_event_pump(),
            (*event_log_store).clone(),
        )));
        // A structured edit claims its file for the effort it happened in.
        event_pump.register_async(Arc::new(tool_call_reactors::EffortClaimConsumer::new(
            efforts.without_event_pump(),
            db.clone(),
            layout.project_dir.clone(),
        )));
        // Every capability's implementations (`capabilities`): core's (the
        // VCS here, knowledge once it's built), what the project's
        // extensions declare, and running provider instances'.
        let capabilities = Arc::new(capabilities::CapabilityRegistry::new(
            vec![capabilities::Implementation {
                capability: "vcs".into(),
                id: vcs.rev_kind().into(),
                title: vcs.rev_kind().into(),
                extension: None,
                source: capabilities::Source::Core,
                features: serde_json::to_value(vcs.features()).unwrap_or(serde_json::Value::Null),
                fields: serde_json::Value::Array(Vec::new()),
                id_pattern: None,
            }],
            vocabulary.clone(),
        ));
        let declared = capabilities::declared_by(&extension_catalog.get(&layout.project_dir));
        capabilities.set_declared(declared.clone());
        // What's active, published before anything reads it: the work-item
        // interface shows the active list's items.
        capabilities
            .publish_now(&config_service::read_config(&config_arc), &db)
            .map_err(|e| AppInitError::Capabilities(e.to_string()))?;
        // The vocabulary reads the active work list's own ids in text
        // (`tsk42`), as that list declares them; from the first read, and
        // at every rebuild (a switch rebuilds it).
        let work_item_ids: vocabulary_reactor::WorkItemIds = {
            let (capabilities, config) = (capabilities.clone(), config_arc.clone());
            Arc::new(move || capabilities.work_item_ids(&config_service::read_config(&config)))
        };
        {
            // Core's still (the service adds extensions' at its first pass).
            let mut core = oxplow_domain::vocabulary::Vocabulary::core();
            core.kinds =
                vocabulary_reactor::with_work_item_ids(core.kinds, work_item_ids().as_ref());
            vocabulary.swap(core);
        }
        let vocabulary_service = Arc::new(vocabulary_reactor::VocabularyService::new(
            db.clone(),
            extension_catalog.clone(),
            layout.project_dir.clone(),
            vocabulary.clone(),
            work_item_ids,
        ));
        let agent_policy = Arc::new(agent_policy::AgentPolicy);
        let commands = Arc::new(
            commands::CommandBus::new(
                db.clone(),
                (*event_log_store).clone(),
                agent_policy.clone(),
                event_pump.clone(),
            )
            // Only a stream's writer thread may change state; an unknown
            // thread may not.
            .with_write_gate({
                let threads = thread_store.clone();
                Arc::new(move |thread| {
                    let threads = threads.clone();
                    Box::pin(async move {
                        use oxplow_domain::stores::ThreadStore as _;
                        matches!(threads.get(&thread).await, Ok(Some(t)) if t.status.is_writer())
                    })
                })
            })
            // Offered and run only while what it needs is active.
            .with_capabilities(capabilities.clone(), config_arc.clone()),
        );
        // Composition (P6b.A1): several Tx commands as one run.
        commands
            .register(commands::compose::sequence_command(&commands))
            .expect("command.sequence registers");
        // The work-items providers (`.context/work-items.md`); oxplow's
        // own, over this bus. Its active one is resolved from the config as
        // it is now.
        let work_items = {
            let (config, capabilities) = (config_arc.clone(), capabilities.clone());
            oxplow_domain::work_items::WorkItemsRegistry::new(Arc::new(move || {
                capabilities.active(&config_service::read_config(&config), "work_items")
            }))
        };
        // The built-in lists the project's extensions declare (oxplow's
        // tasks, while `oxplow-bundled` does).
        work_items::register_built_ins(&work_items, &declared, &db);
        // None as a work list: the sink every verb reaches while no list is
        // active (`work_items::none_provider`).
        work_items.register(work_items::none_provider());
        // A turn's end take becomes a `thread.checkpoint` a policy reads.
        event_pump.register_async(Arc::new(thread_checkpoint::ThreadCheckpointConsumer {
            log: (*event_log_store).clone(),
            sql: sql.clone(),
        }));
        // Its turn's changed files become the effort's observed ones.
        event_pump.register_async(Arc::new(effort_observation::EffortObservationConsumer {
            efforts: effort_store.clone(),
            snapshots: snapshot_store.clone(),
            sql: sql.clone(),
            lifecycle: efforts.without_event_pump(),
        }));
        // The project's effort policy reacts to items starting and
        // finishing, through this bus (`.context/work-tracking.md`).
        event_pump.register_async(Arc::new(effort_policy::EffortPolicyConsumer {
            bus: Arc::downgrade(&commands),
            sql: sql.clone(),
            config: config_arc.clone(),
            capabilities: capabilities.clone(),
        }));
        for command in commands::vcs::ops(commands::vcs::VcsTarget {
            vcs: vcs.clone(),
            git: vcs::GitProvider,
            worktrees: worktrees.clone(),
            events: event_bus.clone(),
            ref_moves: ref_moves.clone(),
        }) {
            commands.add_op(command).expect("vcs ops register");
        }
        let acp = Arc::new(acp::manager::AcpManager::new());
        let link_deps = link_check::LinkDeps {
            project_dir: layout.project_dir.clone(),
            vcs: vcs.clone(),
            db: db.clone(),
            vocabulary: vocabulary.clone(),
        };
        for command in [
            commands::work_item::transition_op(work_items.clone()),
            commands::work_item::update_op(work_items.clone(), link_deps.clone()),
            commands::work_item::create_op(work_items.clone(), link_deps.clone()),
            commands::work_item::link_op(work_items.clone()),
            commands::work_item::comment_op(work_items.clone()),
            commands::work_item::reorder_op(work_items.clone()),
            commands::work_item::move_op(work_items.clone()),
            commands::work_item::delete_op(work_items.clone()),
        ]
        .into_iter()
        .chain(commands::review::ops())
        .chain(commands::thread::ops(config_arc.clone(), acp.clone()))
        .chain(commands::effort::ops(work_items.clone()))
        .chain(commands::hint::ops())
        .chain(commands::dashboard::ops(db.clone(), sql.clone()))
        .chain(commands::comment::ops())
        .chain(commands::reasoning::ops())
        .chain(commands::ui::ops())
        .chain(commands::effort_report::ops(
            commands::effort_report::EffortDeps {
                lifecycle: efforts.clone(),
                efforts: effort_store.clone(),
                sql: sql.clone(),
                db: db.clone(),
                vocabulary: vocabulary.clone(),
                project_dir: layout.project_dir.clone(),
                vcs: vcs.clone(),
            },
        ))
        .chain(commands::note::ops(link_deps.clone()))
        .chain(commands::stream::ops(commands::stream::StreamDeps {
            streams: streams.clone(),
            snapshot_captures: snapshot_captures.clone(),
            ref_moves: ref_moves.clone(),
            threads: thread_store.clone(),
            sessions: agent_session_store.clone(),
            log: event_log_store.clone(),
            search: search_store.clone(),
            worktrees: worktrees.clone(),
            efforts: effort_store.clone(),
        })) {
            commands.add_op(command).expect("core ops register");
        }
        let providers = providers::ProviderRegistry::new(
            providers::HostDeps {
                project_dir: layout.project_dir.clone(),
                project: collector_runner::project_key(&layout.project_dir),
                approvals: approvals.clone(),
                secrets: machine.secrets.clone(),
                config: config_arc.clone(),
                catalog: extension_catalog.clone(),
                db: db.clone(),
                log: (*event_log_store).clone(),
                host_env: machine.host_env.clone(),
                backoff: machine.provider_backoff,
                copies: machine.provider_copies.clone(),
                call_timeout: machine.provider_call_timeout,
                global_dir: machine.config_dir.clone(),
                events: event_bus.clone(),
                capabilities: capabilities.clone(),
            },
            &commands,
            work_items.clone(),
        );
        // An extension's provider commands run on its instances.
        commands.set_provider_router({
            let router: Arc<dyn commands::ProviderRouter> = providers.clone();
            Arc::downgrade(&router)
        });
        let plugin_health =
            plugin_health::PluginHealth::new(db.clone(), event_log_store.vocabulary().clone());
        commands
            .add_op(plugin_health::enable_op(
                plugin_health.clone(),
                Arc::downgrade(&providers),
            ))
            .expect("plugin.enable's op registers");
        commands
            .add_op(providers::sync::sync_op(&providers))
            .expect("provider.sync's op registers");
        let extension_commands = Arc::new(extension_commands::ExtensionCommands::new(
            &commands,
            extension_catalog.clone(),
            layout.project_dir.clone(),
        ));
        let collection = collection::CollectionService::new(
            fact_store.clone(),
            nudge_store.clone(),
            effort_store.clone(),
            thread_store.clone(),
            snapshot_store.clone(),
            snapshot_captures.clone(),
            worktrees.clone(),
            snapshot_content.clone(),
            vcs.clone(),
            config_arc.clone(),
            metric_engine.clone(),
        )
        .with_approvals(approvals.clone())
        .with_vocabulary(vocabulary.clone())
        .with_run_log(collector_runner::RunLog {
            db: db.clone(),
            vocabulary: event_log_store.vocabulary().clone(),
            layer: sql.clone(),
        });
        let collector_runner = collector_runner::CollectorRunner {
            project_dir: layout.project_dir.clone(),
            approvals: approvals.clone(),
            store: collector_store.clone(),
            db: db.clone(),
            vocabulary: event_log_store.vocabulary().clone(),
            secrets: machine.secrets.clone(),
            layer: sql.clone(),
            catalog: extension_catalog.clone(),
            ai: ai_compute.clone(),
            worktrees: worktrees.clone(),
            metrics: metrics.clone(),
            collection: collection.clone(),
        };
        commands
            .add_op(collector_runner::sync_op(collector_runner.clone()))
            .expect("collector.sync's op registers");
        for command in commands::lens::ops(commands::lens::LensTarget {
            project_dir: layout.project_dir.clone(),
            catalog: extension_catalog.clone(),
            db: db.clone(),
            sql: sql.clone(),
        }) {
            commands.add_op(command).expect("lens ops register");
        }
        let thread_answer_store = oxplow_db::SqliteThreadAnswerStore::new(db.clone());
        let panel_layout_store = oxplow_db::SqlitePanelLayoutStore::new(db.clone());
        let knowledge: Arc<dyn oxplow_domain::knowledge::KnowledgeProvider> =
            Arc::new(knowledge::OxplowKnowledge::new(&commands, db.clone()));
        capabilities.add_core(capabilities::Implementation {
            capability: "knowledge".into(),
            id: knowledge.provider().into(),
            title: knowledge.provider().into(),
            extension: None,
            source: capabilities::Source::Core,
            features: serde_json::json!({}),
            fields: serde_json::Value::Array(Vec::new()),
            id_pattern: None,
        });
        for command in knowledge::ops(knowledge::KnowledgeTarget {
            project_dir: layout.project_dir.clone(),
            vcs: vcs.clone(),
        }) {
            commands.add_op(command).expect("knowledge ops register");
        }
        let config_applied = Arc::new(tokio::sync::Notify::new());
        let config_target = commands::config_commands::ConfigTarget {
            config: config_arc.clone(),
            project_dir: layout.project_dir.clone(),
            events: event_bus.clone(),
            applied: config_applied.clone(),
        };
        for command in commands::config_commands::ops(config_target.clone())
            .into_iter()
            .chain(commands::metric::ops(commands::metric::MetricTarget {
                config: config_target,
                metrics: metrics.clone(),
                facts: fact_store.clone(),
                primary_stream: primary_stream.id,
            }))
        {
            commands.add_op(command).expect("core ops register");
        }
        for command in commands::test_runs::ops(collection.clone())
            .into_iter()
            .chain(commands::extension_install::ops(
                commands::extension_install::InstallDeps {
                    worktrees: worktrees.clone(),
                },
            ))
            .chain([commands::snapshot::restore_file_op(
                snapshot_files::SnapshotFiles {
                    snapshots: snapshot_store.clone(),
                    streams: stream_store.clone(),
                    content: snapshot_content.clone(),
                    project_dir: layout.project_dir.clone(),
                },
            )])
            .chain(commands::lsp::ops(commands::lsp::LspDeps {
                installer: lsp_installer_svc.clone(),
                background: background_tasks.clone(),
                events: event_bus.clone(),
            }))
        {
            commands.add_op(command).expect("core ops register");
        }
        let effect_services = commands::effect::ServicesSlot::default();
        let client_host = Arc::new(client_host::ClientHost::new(event_bus.clone()));
        // oxplow's own commands are declared in its required extensions
        // over these operations (`commands/ops.rs`).
        for op in commands::bookmark::ops()
            .into_iter()
            .chain(commands::effect::ops(effect_services.clone()))
            .chain(client_host::ops(&client_host))
        {
            commands.add_op(op).expect("core ops register");
        }
        extension_commands::register_required(&commands).expect("oxplow's own commands register");
        let token_usage_store = Arc::new(SqliteTokenUsageStore::new(db.clone()));
        let token_usage = token_usage::TokenUsageService::new(
            token_usage_store.clone(),
            effort_store.clone(),
            thread_store.clone(),
            agent_session_store.clone(),
            fact_store.clone(),
        );
        let otlp_ingest =
            otlp_ingest::OtlpIngestService::new(db.clone(), vocabulary.clone(), event_pump.clone());

        let advisories = Arc::new(advisories::AdvisoryRunner::new((*nudge_store).clone()));
        // State entity metrics re-capture as their rows move (P7.B6).
        event_pump.register_async(Arc::new(metrics_service::EntityStates {
            metrics: metrics.clone(),
        }));
        // A `generated` change reaches the snapshot captures (tsk515).
        event_pump.register_async(Arc::new(config_reactors::WorkspaceFilterConsumer {
            captures: snapshot_captures.clone(),
            project_dir: layout.project_dir.clone(),
        }));
        // An agent's telemetry export is counted once it is logged (P10.M2).
        event_pump.register_async(Arc::new(token_usage::OtlpTokensConsumer {
            tokens: token_usage.clone(),
        }));
        // A turn's tokens are counted when it ends (P3.7).
        event_pump.register_async(Arc::new(token_usage::TurnTokensConsumer {
            tokens: token_usage.clone(),
            turns: oxplow_db::SqliteAgentTurnStore::with_vocabulary(db.clone(), vocabulary.clone()),
        }));
        // Collection and post-tool advisories react to finished tool calls
        // on the pump (P3.6).
        event_pump.register_async(Arc::new(post_tool_reactors::CollectionConsumer {
            collection: collection.clone(),
            db: db.clone(),
        }));
        // A run reported any other way has its coverage read from its event.
        event_pump.register_async(Arc::new(post_tool_reactors::RunReportsConsumer {
            collection: collection.clone(),
        }));
        let advisory_deps = advisories::AdvisoryDeps {
            advisories: advisories.clone(),
            effort_store: effort_store.clone(),
            thread_store: thread_store.clone(),
            worktrees: worktrees.clone(),
            approvals: approvals.clone(),
            extension_catalog: extension_catalog.clone(),
            db: db.clone(),
            sql: sql.clone(),
            collection: collection.clone(),
            capabilities: capabilities.clone(),
            config: config_arc.clone(),
        };
        event_pump.register_async(Arc::new(post_tool_reactors::PostToolAdvisories {
            deps: advisory_deps.clone(),
        }));
        // Hints a turn's end raises, for the agent's next prompt.
        event_pump.register_async(Arc::new(advisories::TurnEndAdvisories {
            deps: advisory_deps,
        }));
        Ok(Self {
            config: config_arc,
            config_applied,
            db,
            layout,
            streams,
            threads,
            efforts,
            snapshot_captures,
            stream_store,
            thread_store,
            agent_session_store,
            task_store,
            thread_note_store,
            task_link_store,
            event_log_store,
            sql,
            model_watermarks,
            assets,
            vocabulary,
            event_pump,
            extension_models,
            vocabulary_service,
            extension_catalog,
            component_bundles: Arc::new(component_bundles::ComponentBundles::new()),
            extension_commands,
            effect_services,
            client_host,
            commands,
            work_items,
            capabilities,
            providers,
            knowledge,
            wiki_page_store,
            page_visit_store,
            usage_store,
            code_quality_store,
            snapshot_store,
            search_store,
            agent_status_store,
            agent_turn_store,
            thread_runtime,
            effort_store,
            fact_store,
            metric_engine,
            metric_visibility,
            metrics,
            nudge_store,
            dashboard_store,
            collector_store,
            collector_runner,
            thread_answer_store,
            panel_layout_store,
            reasoning_store,
            tool_call_store,
            git_store,
            diagnostic_store,
            symbol_store,
            ai,
            ai_compute,
            secrets: machine.secrets,
            effort_evidence_store,
            advisories,
            change_store,
            change_analyzer: Arc::new(change_analysis::ChangeAnalyzer::default()),
            collection,
            token_usage_store,
            token_usage,
            otlp_ingest,
            wiki_page_thread_updates,
            page_ref_store,
            comment_store,
            hook_ingest,
            background_tasks,
            followups,
            pty,
            blobs,
            snapshot_content,
            code_intel,
            lsp_sessions: lsp,
            agent_policy,
            agent_context: Arc::new(agent_context::AgentContext::default()),
            acp,
            approvals,
            lsp_installer: lsp_installer_svc,
            terminal_sessions,
            output_activity,
            recovery: recovery_svc,
            events: event_bus,
            git,
            vcs,
            worktrees,
            workspace_files,
            branch_reconciler,
            ref_moves,
            trees,
        })
    }

    /// Test-only constructor with an in-memory DB. Useful for the
    /// IPC layer's smoke tests where we want a real Services without
    /// hitting the filesystem.
    pub fn in_memory(project_dir: impl Into<PathBuf>) -> Result<Self, AppInitError> {
        let project_dir = project_dir.into();
        let global = project_dir.join(".oxplow/global-config");
        Self::in_memory_on_machine(
            project_dir,
            global,
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            providers::host::process_env(),
        )
    }

    /// [`Self::in_memory`] for one of several projects on one machine:
    /// `global_dir`, `secrets` and `host_env` stand in for the machine's
    /// global config dir, keychain and environment, shared by every
    /// project built on them.
    pub fn in_memory_on_machine(
        project_dir: impl Into<PathBuf>,
        global_dir: PathBuf,
        secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
        host_env: providers::host::HostEnv,
    ) -> Result<Self, AppInitError> {
        let project_dir = project_dir.into();
        let state_dir = project_dir.join(".oxplow");
        ensure_state_dir(&state_dir)?;
        let layout = AppLayout {
            project_dir: project_dir.clone(),
            state_dir: state_dir.clone(),
            state_db_path: state_dir.join("local.sqlite"),
        };
        let config = oxplow_config::load_project_config(&project_dir)?;
        // Tests never touch the real keychain or the user's ai.yaml.
        let machine = MachineEnv {
            secrets,
            config_dir: Some(global_dir.clone()),
            approvals_file: Some(global_dir.join("approvals.json")),
            // A test's failing provider restarts at once.
            provider_backoff: std::time::Duration::ZERO,
            provider_copies: global_dir.join("provider-copies"),
            // A test's hung provider fails fast.
            provider_call_timeout: std::time::Duration::from_secs(2),
            lsp_request_timeout: std::time::Duration::from_secs(2),
            host_env,
        };
        Self::build(layout, config, Database::in_memory(), machine)
    }

    /// Reload `.oxplow/project.yaml` from disk into the in-memory config, re-apply
    /// derived state (the snapshot workspace filter, mirroring
    /// `set_generated`), and emit `ConfigChanged`. Called by the config
    /// fs-watcher when the file changes out-of-band — e.g. the agent
    /// running `/oxplow:configure` writes a `testing:` block — so the
    /// edit goes live without a process restart. A full reload from disk
    /// is safe: the in-memory config is always exactly the file's content
    /// plus defaults (the same thing boot computes).
    pub fn reload_config_from_disk(&self) -> Result<(), AppInitError> {
        let fresh = oxplow_config::load_project_config(&self.layout.project_dir)?;
        let filter = oxplow_fs_watch::WorkspaceFilter::for_project(
            &self.layout.project_dir,
            &fresh.generated.exclude,
            &fresh.generated.include,
        );
        {
            let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
            *guard = fresh;
        }
        self.snapshot_captures.set_workspace_filter(filter);
        self.config_applied.notify_waiters();
        self.events.emit(OxplowEvent::ConfigChanged);
        Ok(())
    }
}

/// Forward every BackgroundTaskStore broadcast event onto the typed
/// EventBus as `OxplowEvent::BackgroundTasksChanged`. The store's own
/// channel carries finer-grained `Started`/`Updated`/`Ended` info, but
/// the renderer's coarse `backgroundTasksChanged` listener (re-fetches
/// the row and decides terminal vs non-terminal from `status`) is
/// sufficient. Without this bridge the bottom-bar indicator stays
/// silent and `awaitBackgroundTask` never resolves.
/// A thread's in-memory follow-ups changed: the renderer re-reads that
/// thread's (tsk789).
fn bridge_followup_events(store: &FollowupStore, bus: &EventBus) {
    let mut rx = store.subscribe();
    let bus = bus.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(thread_id) => bus.emit(OxplowEvent::FollowupsChanged { thread_id }),
                // Missed some: which threads isn't known, so nothing to
                // name; the next change reaches its thread.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn bridge_background_task_events(store: &BackgroundTaskStore, bus: &EventBus) {
    let mut rx = store.subscribe();
    let bus = bus.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(_) => {
                    bus.emit(OxplowEvent::BackgroundTasksChanged);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // A laggy subscriber missed some events; emit one
                    // catch-up tick so the UI re-fetches.
                    bus.emit(OxplowEvent::BackgroundTasksChanged);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// tsk789: a follow-up added or removed reaches the renderer as
    /// `FollowupsChanged` for its thread.
    #[tokio::test]
    async fn a_followup_change_reaches_the_bus() {
        let (store, bus) = (FollowupStore::new(), EventBus::new());
        let mut ui = bus.subscribe_ui();
        bridge_followup_events(&store, &bus);
        let thread = oxplow_domain::ThreadId::new(4);
        store.add(thread, "later".into());
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), ui.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(got, OxplowEvent::FollowupsChanged { thread_id } if thread_id == thread),
            "{got:?}"
        );
    }

    #[tokio::test]
    async fn boot_creates_state_dir() {
        let project = tempdir().unwrap();
        // Init a git repo so session validation passes for any
        // future calls that go through StreamService.
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let layout = AppLayout::for_project(project.path());
        let services = Services::boot(
            layout,
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
        .unwrap();
        assert!(services.layout.state_dir.exists());
        assert!(services.layout.state_db_path.exists());
    }

    #[tokio::test]
    async fn in_memory_does_not_touch_disk_db() {
        let project = tempdir().unwrap();
        // ensure_primary refuses non-git dirs, so init a repo first.
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let services = Services::in_memory(project.path()).unwrap();
        // The state dir is created (config load needs it for fallback
        // basename) but the DB is in-memory.
        assert!(services.layout.state_dir.exists());
        // Writing to db should be fine; the file path will not exist.
        assert!(!services.layout.state_db_path.exists());
    }

    fn init_git(dir: &std::path::Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    #[tokio::test]
    async fn reload_config_from_disk_picks_up_external_edits() {
        let project = tempdir().unwrap();
        init_git(project.path());
        let services = Services::in_memory(project.path()).unwrap();
        // Boot config reads no reports.
        assert!(config_service::read_config(&services.config)
            .collectors
            .is_empty());

        // Simulate `/oxplow:configure` writing the file out-of-band.
        std::fs::write(
            oxplow_config::config_path(project.path()),
            "collectors:\n  - { id: tests.coverage, records: coverage, entry: \"oxplow:lcov\", report: { path: target/coverage/lcov.info }, trigger: { on_run: test } }\n",
        )
        .unwrap();

        services.reload_config_from_disk().unwrap();

        let cfg = config_service::read_config(&services.config);
        assert_eq!(
            cfg.collectors.len(),
            1,
            "in-memory config should reflect the on-disk edit after reload"
        );
        assert_eq!(
            cfg.collectors[0].report.as_ref().unwrap().path,
            "target/coverage/lcov.info"
        );
    }

    /// P5.A1 (tsk519): every `External` command — one that runs outside
    /// the bus's transaction, against a system the bus doesn't own — is
    /// listed here on purpose. Adding one means deciding that its system
    /// owns the state, and saying which system in its summary.
    #[tokio::test]
    async fn the_external_commands_are_the_reviewed_ones() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let services = Services::in_memory(dir.path()).unwrap();
        assert_eq!(
            services.commands.external_commands(),
            [
                // The collector's own program or script (P7.B3).
                "oxplow.collector.sync",
                // A tile's SQL is checked by the semantic engine first (P8.A5).
                "oxplow.dashboard.add_item",
                "oxplow.dashboard.update_item",
                // The worktree and the snapshot diff a report is checked
                // against (P8.A7).
                "oxplow.effect.backfill",
                "oxplow.effect.retry",
                "oxplow.effort.report",
                // A clone into a stream's worktree, a person's call (P8.A9).
                "oxplow.extension.install",
                "oxplow.extension.update",
                "oxplow.git.cherry_pick",
                "oxplow.git.ignore",
                "oxplow.git.rebase",
                "oxplow.git.revert",
                // Lens files on disk (P6 review, tsk597): a Tx handler may
                // run twice, and a retried file write strands the first.
                "oxplow.lens.keep",
                "oxplow.lens.share",
                // A download into .oxplow/lsp/, a person's call (P8.A9).
                "oxplow.lsp.install_server",
                "oxplow.lsp.remove_server",
                "oxplow.metric.rebuild",
                // A provider's process restarts (P7.C1; was provider.enable).
                "oxplow.plugin.enable",
                // The provider process's collectors (P7.A3).
                "oxplow.provider.sync",
                // Overwrites a worktree file (P8.A9).
                "oxplow.snapshot.restore_file",
                // A stream's worktree and its capture service (P8.A4).
                "oxplow.stream.adopt_worktree",
                "oxplow.stream.archive",
                "oxplow.stream.create_worktree",
                // A run's capture, through the collection service (P8.A8).
                "oxplow.test.record_run",
                "oxplow.vcs.checkout_branch",
                "oxplow.vcs.commit",
                "oxplow.vcs.delete_branch",
                "oxplow.vcs.discard",
                "oxplow.vcs.fetch",
                "oxplow.vcs.merge",
                "oxplow.vcs.pull",
                "oxplow.vcs.push",
                "oxplow.vcs.rename_branch",
                "oxplow.vcs.resolve_conflict",
                "oxplow.vcs.stage",
                // Every work list's verbs, oxplow's own included.
                "oxplow.work_item.comment",
                "oxplow.work_item.create",
                "oxplow.work_item.delete",
                "oxplow.work_item.link",
                "oxplow.work_item.move",
                "oxplow.work_item.reorder",
                "oxplow.work_item.transition",
                "oxplow.work_item.update",
            ]
        );
    }

    /// Every composite — whose calls decide, per input, whether it runs in
    /// the transaction or as steps — is listed here on purpose, like the
    /// `External` commands.
    #[tokio::test]
    async fn the_composite_commands_are_the_reviewed_ones() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let services = Services::in_memory(dir.path()).unwrap();
        assert_eq!(
            services.commands.composite_commands(),
            ["oxplow.command.sequence",]
        );
    }
}
