//! Application services / use-cases layer.
//!
//! Constructs the dependency graph: Database → store impls →
//! services. The Tauri command crate and the MCP crate both call into
//! this layer; they never reach into infrastructure crates directly.
//!
//! Held inside `Arc<Services>` and registered as Tauri state. Methods
//! on `Services` are the high-level "use cases" the IPC layer calls.

pub mod agent_command;
pub mod agent_pane;
pub mod agent_prompt;
pub mod agent_stall_watch;
pub mod agent_status_derive;
pub mod background_task;
pub mod blob_store;
pub mod boot;
pub mod code_quality_runner;
pub mod collection;
pub mod commit_indexer;
pub mod config_service;
pub mod config_watch;
pub mod diagnostics;
pub mod events;
pub mod file_ref_version;
pub mod followup;
pub mod git_service;
pub mod hook_ingest;
pub mod indexer;
pub mod lsp_installer;
pub mod lsp_sessions;
pub mod output_activity;
pub mod page_ref_backfill;
pub mod recovery;
pub mod ref_resolver;
pub mod resume_check;
pub mod snapshot_capture;
pub mod snapshot_capture_registry;
pub mod snapshot_content;
pub mod task_service;
pub mod terminal_sessions;
pub mod thread_runtime;
pub mod token_usage;
pub mod wiki_drift;
pub mod wiki_pages;
pub mod wiki_pages_watch;
pub mod workspace_watch;

pub use agent_prompt::{
    build_session_context_block, build_session_context_block_with_role, role_change_banner,
    RoleMode,
};
pub use events::{
    event_channels, CodeQualityScanPhase, EventBus, OxplowEvent, SnapshotSourceKind,
    WorkspaceChangeKind,
};
pub use hook_ingest::{HookEnvelope, HookIngestError, HookIngestService};
pub use oxplow_lsp::{LspError, LspProxy};
pub use task_service::{
    BacklogState, CreateTaskInput, TaskService, TaskServiceError, UpdateTaskChanges,
};

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
pub use oxplow_db::{EffortFile, TaskEffortStore};

use thiserror::Error;
use tracing::info;

use std::sync::RwLock;

use oxplow_config::OxplowConfig;
use oxplow_db::{
    Database, SqliteAgentNudgeStore, SqliteAgentTurnStore, SqliteCodeQualityStore,
    SqliteCommentStore, SqliteEffortObservationStore, SqlitePageRefStore, SqlitePageVisitStore,
    SqliteSearchStore, SqliteSnapshotStore, SqliteStreamStore, SqliteTaskEffortStore,
    SqliteTaskEventStore, SqliteTaskLinkStore, SqliteTaskNoteStore, SqliteTaskStore,
    SqliteThreadStore, SqliteTokenUsageStore, SqliteUsageStore, SqliteWikiPageStore,
    SqliteWikiPageThreadUpdateStore,
};
use oxplow_domain::stores::{AgentStatusStore, HookEventStore};
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
        let state_db_path = state_dir.join("state.sqlite");
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

/// Ensure the `.oxplow/` state directory exists and is self-ignoring.
///
/// Creates `state_dir` (recursively) and drops a `.gitignore` holding a
/// single `*` so the whole directory — including the gitignore itself —
/// is excluded from the host repo without touching the project's root
/// `.gitignore`. Idempotent: an existing `.gitignore` is left untouched.
pub fn ensure_state_dir(state_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let gitignore = state_dir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(gitignore, "*\n")?;
    }
    Ok(())
}

/// Try to take the per-project single-instance lock. On success the
/// held [`std::fs::File`] is returned — keep it alive for the whole
/// process (the OS releases the advisory lock when it drops). `None`
/// means another live oxplow process already holds it, so this process
/// must not boot a second `Services` on the same `state.sqlite`
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

/// Spawn a fresh oxplow process pinned to `dir` — the process-per-
/// window primitive. `OXPLOW_PROJECT_DIR` is what the shell honours at
/// startup; the positional arg is for `ps` visibility. The child
/// detaches and survives the caller's exit. Used by the IPC
/// open/setup commands and by session restore at startup.
///
/// `restoring` marks the child as part of session restore via
/// `OXPLOW_RESTORING`: such a child keeps the existing session set
/// (adds itself) instead of re-snapshotting the live set, so
/// concurrent restore spawns can't clobber each other.
pub fn spawn_project_window(dir: &std::path::Path, restoring: bool) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.env("OXPLOW_PROJECT_DIR", dir).arg(dir);
    if restoring {
        cmd.env("OXPLOW_RESTORING", "1");
    }
    cmd.spawn()?;
    Ok(())
}

/// Non-destructive probe: is `project_dir` currently held by a live
/// oxplow process? Used by `open_project` to warn before spawning a
/// duplicate window. Acquiring the lock here would itself succeed when
/// nobody holds it, so we immediately drop it (releasing) and report
/// the prior state.
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

/// Focus-channel coordinates a running project process publishes so a
/// second launch of the same project can raise its window instead of
/// failing on the instance lock. Written to `.oxplow/instance.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstanceInfo {
    /// Loopback TCP port the running instance listens on for focus pings.
    pub focus_port: u16,
    /// Shared secret echoed back on the focus ping so an unrelated
    /// process that happens to hold the port can't be made to act.
    pub nonce: String,
}

fn instance_info_path(project_dir: &std::path::Path) -> PathBuf {
    project_dir.join(".oxplow").join("instance.json")
}

/// A fresh random focus nonce.
pub fn new_focus_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Publish this process's focus coordinates for `layout`'s project.
pub fn write_instance_info(layout: &AppLayout, info: &InstanceInfo) -> std::io::Result<()> {
    ensure_state_dir(&layout.state_dir)?;
    let json = serde_json::to_vec_pretty(info)?;
    std::fs::write(instance_info_path(&layout.project_dir), json)
}

/// Best-effort: remove the focus coordinates (called as a project
/// process shuts down so a stale port isn't left behind).
pub fn clear_instance_info(project_dir: &std::path::Path) {
    let _ = std::fs::remove_file(instance_info_path(project_dir));
}

fn read_instance_info(project_dir: &std::path::Path) -> Option<InstanceInfo> {
    let bytes = std::fs::read(instance_info_path(project_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Ask the running instance of `project_dir` to focus its window.
/// Returns `true` if the ping was delivered. Only meaningful when the
/// project is actually open (the caller checks [`is_project_locked`]
/// first); a stale/unreachable file just returns `false` so the caller
/// can fall back.
pub fn request_focus(project_dir: &std::path::Path) -> bool {
    use std::io::Write;
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Duration;

    let Some(info) = read_instance_info(project_dir) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.focus_port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let mut line = info.nonce;
    line.push('\n');
    stream.write_all(line.as_bytes()).is_ok()
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

    #[test]
    fn ensure_state_dir_writes_self_ignoring_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".oxplow");

        ensure_state_dir(&state_dir).unwrap();

        assert!(state_dir.is_dir());
        let gitignore = std::fs::read_to_string(state_dir.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "*\n");
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

    #[test]
    fn request_focus_delivers_nonce_to_listener() {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;

        let dir = tempfile::tempdir().unwrap();
        let layout = AppLayout::for_project(dir.path());

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let nonce = new_focus_nonce();
        write_instance_info(
            &layout,
            &InstanceInfo {
                focus_port: port,
                nonce: nonce.clone(),
            },
        )
        .unwrap();

        let want = nonce.clone();
        let handle = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut got = String::new();
            BufReader::new(conn).read_line(&mut got).unwrap();
            assert_eq!(got.trim(), want);
        });

        assert!(request_focus(dir.path()), "ping should be delivered");
        handle.join().unwrap();
    }

    #[test]
    fn request_focus_false_without_instance_info() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!request_focus(dir.path()));
    }
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
    pub db: Database,
    pub layout: AppLayout,
    pub streams: StreamService,
    pub threads: ThreadService,
    pub tasks: TaskService,
    pub stream_store: Arc<SqliteStreamStore>,
    pub thread_store: Arc<SqliteThreadStore>,
    pub task_store: Arc<SqliteTaskStore>,
    pub work_note_store: Arc<SqliteTaskNoteStore>,
    pub task_link_store: Arc<SqliteTaskLinkStore>,
    pub task_event_store: Arc<SqliteTaskEventStore>,
    pub wiki_page_store: Arc<SqliteWikiPageStore>,
    pub page_visit_store: Arc<SqlitePageVisitStore>,
    pub usage_store: Arc<SqliteUsageStore>,
    pub code_quality_store: Arc<SqliteCodeQualityStore>,
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    /// Unified site-wide search index (FTS5/BM25). Written by the
    /// `Indexer` (`indexer.rs`); read by the `search` IPC/MCP surface.
    pub search_store: Arc<SqliteSearchStore>,
    /// Per-stream snapshot capture registry. Holds one service per
    /// active stream (each watching its own worktree) and is the
    /// stream-aware replacement for the singleton above. Callers that
    /// know which stream they're acting on should `get(&stream_id)`
    /// here; legacy callers that need "the primary" use
    /// `snapshot_captures.primary()` (or the `snapshot_capture` alias).
    pub snapshot_captures: snapshot_capture_registry::SnapshotCaptureRegistry,
    pub hook_event_store: Arc<dyn HookEventStore>,
    pub agent_status_store: Arc<dyn AgentStatusStore>,
    pub agent_turn_store: Arc<SqliteAgentTurnStore>,
    /// Backing in-memory state for hook events + agent status. Both
    /// `hook_event_store` and `agent_status_store` are trait-object
    /// views of this same registry — keep the concrete handle around
    /// for code that wants to bypass the trait surfaces.
    pub thread_runtime: Arc<thread_runtime::ThreadRuntimeRegistry>,
    pub effort_store: Arc<SqliteTaskEffortStore>,
    /// Effort-scoped collection observations (test runs + diff coverage).
    pub observation_store: Arc<SqliteEffortObservationStore>,
    /// Persisted agent nudges (report-less-run / commit-hygiene) — the
    /// human-facing record of what oxplow steered the agent to do.
    pub nudge_store: Arc<SqliteAgentNudgeStore>,
    /// Collection engine (passive Bash-hook detection + coverage ingest).
    pub collection: collection::CollectionService,
    /// Per-turn agent token usage parsed from the hook transcript (tsk104).
    pub token_usage_store: Arc<SqliteTokenUsageStore>,
    /// Captures token usage on Stop from the agent transcript.
    pub token_usage: token_usage::TokenUsageService,
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
    pub tmux: Arc<dyn oxplow_tmux::TmuxRunner>,
    pub agent_panes: agent_pane::AgentPaneService,
    pub blobs: blob_store::BlobStore,
    pub lsp_sessions: lsp_sessions::LspSessionManager,
    pub lsp_installer: lsp_installer::LspInstallerService,
    pub terminal_sessions: terminal_sessions::TerminalSessionRegistry,
    /// Shared per-thread PTY liveness, written by the terminal forwarder
    /// and read by the agent stall watchdog (tsk141).
    pub output_activity: output_activity::OutputActivity,
    pub recovery: recovery::RecoveryService,
    pub events: EventBus,
    /// Singleton git access surface — every read of git state and
    /// every mutating git op routes through here so we can layer
    /// caching in one place. See `git_service.rs`.
    pub git: Arc<git_service::GitService>,
    /// Per-thread cursor for the rail's "Recently finished" section.
    /// Entries whose timestamp is `<= cursor` are filtered out. Keyed
    /// by thread id; entries with no thread (global view) live under
    /// the empty string. In-memory only — clearing the section is a UX
    /// gesture, not a destructive op, and re-appearing after a restart
    /// is fine.
    pub finished_cleared_at:
        Arc<RwLock<std::collections::HashMap<String, oxplow_domain::Timestamp>>>,
}

impl Services {
    /// Bootstrap. Run once at app startup.
    pub fn boot(layout: AppLayout) -> Result<Self, AppInitError> {
        ensure_state_dir(&layout.state_dir)?;

        let config = oxplow_config::load_project_config(&layout.project_dir)?;
        info!(project = %layout.project_dir.display(), agents = ?config.agents, "config loaded");

        let db = Database::open(&layout.state_db_path)?;
        Self::build(layout, config, db)
    }

    /// Shared construction core for [`Self::boot`] and
    /// [`Self::in_memory`]: every store, service, and registry is
    /// wired here, in dependency order, exactly once. The two
    /// entrypoints differ only in how they resolve the layout,
    /// config, and `Database` handle.
    fn build(layout: AppLayout, config: OxplowConfig, db: Database) -> Result<Self, AppInitError> {
        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
        let page_ref_store = Arc::new(SqlitePageRefStore::new(db.clone()));
        let comment_store = Arc::new(SqliteCommentStore::new(db.clone()));
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let work_note_store = Arc::new(SqliteTaskNoteStore::new(db.clone()));
        let task_link_store = Arc::new(SqliteTaskLinkStore::new(db.clone()));
        let task_event_store = Arc::new(SqliteTaskEventStore::new(db.clone()));
        let wiki_page_store = Arc::new(SqliteWikiPageStore::new(db.clone()));
        let page_visit_store = Arc::new(SqlitePageVisitStore::new(db.clone()));
        let usage_store = Arc::new(SqliteUsageStore::new(db.clone()));
        let code_quality_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let snapshot_store = Arc::new(SqliteSnapshotStore::new(db.clone()));
        let search_store = Arc::new(SqliteSearchStore::new(db.clone()));
        let thread_runtime =
            Arc::new(thread_runtime::ThreadRuntimeRegistry::with_default_capacity());
        let hook_event_store: Arc<dyn HookEventStore> = thread_runtime.clone();
        let agent_status_store: Arc<dyn AgentStatusStore> = thread_runtime.clone();
        let agent_turn_store = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let effort_store = Arc::new(SqliteTaskEffortStore::new(db.clone()));
        let observation_store = Arc::new(SqliteEffortObservationStore::new(db.clone()));
        let nudge_store = Arc::new(SqliteAgentNudgeStore::new(db.clone()));
        let wiki_page_thread_updates = Arc::new(SqliteWikiPageThreadUpdateStore::new(db.clone()));

        let workspace_layout = WorkspaceLayout::for_project(&layout.project_dir);
        let streams =
            StreamService::new(workspace_layout, stream_store.clone(), thread_store.clone());
        let threads = ThreadService::new(thread_store.clone());
        let tasks = TaskService::new(task_store.clone());
        let event_bus = EventBus::new();
        let hook_ingest = HookIngestService::new(
            hook_event_store.clone(),
            agent_status_store.clone(),
            agent_turn_store.clone(),
            event_bus.clone(),
        );
        let recovery_svc = recovery::RecoveryService::new(
            agent_turn_store.clone(),
            task_store.clone(),
            effort_store.clone(),
            event_bus.clone(),
        );

        let pty = oxplow_pty::PtyManager::spawn();
        let tmux: Arc<dyn oxplow_tmux::TmuxRunner> = Arc::new(oxplow_tmux::SystemTmux::new());
        let agent_panes = agent_pane::AgentPaneService::new(tmux.clone());
        // Lazily-built per-(stream, language) LSP proxies. Spawn cost
        // is paid on first request, not at boot.
        let config_arc = Arc::new(RwLock::new(config));
        let lsp = lsp_sessions::LspSessionManager::new(config_arc.clone());
        let lsp_installer_svc =
            lsp_installer::LspInstallerService::new(&layout.state_dir, lsp.clone());
        if let Err(e) = futures::executor::block_on(lsp_installer_svc.replay_into_sessions()) {
            tracing::warn!(?e, "lsp installer manifest replay failed");
        }
        // Shared PTY liveness — the terminal forwarder stamps it, the
        // stall watchdog reads it (tsk141).
        let output_activity = output_activity::OutputActivity::new();
        let terminal_sessions = terminal_sessions::TerminalSessionRegistry::new(
            pty.clone(),
            tmux.clone(),
            output_activity.clone(),
        );
        let blobs = blob_store::BlobStore::new(layout.state_dir.join("snapshots"));
        let git = git_service::GitService::spawn(
            layout.project_dir.clone(),
            stream_store.clone(),
            event_bus.clone(),
        );

        // Snapshot capture singleton — owned here so anything in
        // Services can request snapshots (e.g. TaskService stamps
        // start/end ids on the effort row when a task transitions
        // through in_progress). The fs-watcher, startup sweep, and
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
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: max_bytes,
                workspace_filter,
                events: event_bus.clone(),
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
        // Now that the capture registry exists and its streams are
        // registered, give recovery the wiring to bracket + reconcile
        // orphaned efforts left open by a crash (death/restart case).
        let recovery_svc =
            recovery_svc.with_snapshot_reconcile(thread_store.clone(), snapshot_captures.clone());
        let tasks = tasks
            .with_effort_store(effort_store.clone())
            .with_snapshot_captures(snapshot_captures.clone())
            .with_thread_store(thread_store.clone());
        let collection = collection::CollectionService::new(
            observation_store.clone(),
            nudge_store.clone(),
            effort_store.clone(),
            thread_store.clone(),
            snapshot_store.clone(),
            blobs.clone(),
            config_arc.clone(),
            layout.project_dir.clone(),
            event_bus.clone(),
        );
        let token_usage_store = Arc::new(SqliteTokenUsageStore::new(db.clone()));
        let token_usage = token_usage::TokenUsageService::new(
            token_usage_store.clone(),
            effort_store.clone(),
            thread_store.clone(),
            event_bus.clone(),
        );

        let background_tasks = BackgroundTaskStore::new();
        bridge_background_task_events(&background_tasks, &event_bus);

        Ok(Self {
            config: config_arc,
            db,
            layout,
            streams,
            threads,
            tasks,
            snapshot_captures,
            stream_store,
            thread_store,
            task_store,
            work_note_store,
            task_link_store,
            task_event_store,
            wiki_page_store,
            page_visit_store,
            usage_store,
            code_quality_store,
            snapshot_store,
            search_store,
            hook_event_store,
            agent_status_store,
            agent_turn_store,
            thread_runtime,
            effort_store,
            observation_store,
            nudge_store,
            collection,
            token_usage_store,
            token_usage,
            wiki_page_thread_updates,
            page_ref_store,
            comment_store,
            hook_ingest,
            background_tasks,
            followups: FollowupStore::new(),
            pty,
            tmux,
            agent_panes,
            blobs,
            lsp_sessions: lsp,
            lsp_installer: lsp_installer_svc,
            terminal_sessions,
            output_activity,
            recovery: recovery_svc,
            events: event_bus,
            git,
            finished_cleared_at: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Test-only constructor with an in-memory DB. Useful for the
    /// IPC layer's smoke tests where we want a real Services without
    /// hitting the filesystem.
    pub fn in_memory(project_dir: impl Into<PathBuf>) -> Result<Self, AppInitError> {
        let project_dir = project_dir.into();
        let state_dir = project_dir.join(".oxplow");
        ensure_state_dir(&state_dir)?;
        let layout = AppLayout {
            project_dir: project_dir.clone(),
            state_dir: state_dir.clone(),
            state_db_path: state_dir.join("state.sqlite"),
        };
        let config = oxplow_config::load_project_config(&project_dir)?;
        Self::build(layout, config, Database::in_memory())
    }

    /// Reload `oxplow.yaml` from disk into the in-memory config, re-apply
    /// derived state (the snapshot workspace filter, mirroring
    /// `set_generated`), and emit `ConfigChanged`. Called by the config
    /// fs-watcher when the file changes out-of-band — e.g. the agent
    /// running `/oxplow:configure` writes a `collection:` block — so the
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
        let services = Services::boot(layout).unwrap();
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
        // Boot config has no collection profile.
        assert!(config_service::read_config(&services.config)
            .collection
            .reports
            .is_empty());

        // Simulate `/oxplow:configure` writing the file out-of-band.
        std::fs::write(
            project.path().join(oxplow_config::OXPLOW_CONFIG_FILE),
            "collection:\n  reports:\n    - { path: target/coverage/lcov.info, format: lcov }\n",
        )
        .unwrap();

        services.reload_config_from_disk().unwrap();

        let cfg = config_service::read_config(&services.config);
        assert_eq!(
            cfg.collection.reports.len(),
            1,
            "in-memory config should reflect the on-disk edit after reload"
        );
        assert_eq!(cfg.collection.reports[0].path, "target/coverage/lcov.info");
    }
}
