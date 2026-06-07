// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxplow_app::{AppLayout, BackgroundTaskKind, Services, StartInput, UpdateInput};
use oxplow_tauri_ipc::{
    specta_builder, AppState, PluginRuntime, PluginRuntimeState, OXPLOW_EVENT_CHANNEL,
};
use tauri::Emitter;

/// Set once the app is quitting as a whole (Cmd-Q / app exit). A
/// single window closing while this is false is a deliberate
/// per-window close (drop it from the restore set); during a full
/// quit we preserve the set so every open window comes back.
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() {
    if let Some(event) = hook_event_arg() {
        run_hook_command(&event);
        return;
    }

    init_tracing();

    // `generate_context!` embeds the Info.plist and may expand only
    // once per binary — build it here and hand it to whichever mode
    // we boot into.
    let ctx = tauri::generate_context!();

    // Process-per-window model: this process serves exactly one
    // project, chosen at launch. A bare launch (Finder/Spotlight/dock,
    // no arg + no env) has no project and boots the launcher instead.
    // A dir that isn't an Oxplow project yet (no `.oxplow/`) boots the
    // setup-confirmation screen rather than silently initializing.
    match resolve_project_dir() {
        Some(dir) if dir.join(".oxplow").is_dir() => run_project(dir, ctx),
        Some(dir) => run_setup(dir, ctx),
        // Bare launch: reopen the windows that were open at last exit;
        // if there were none, show the launcher.
        None if restore_session() => {}
        None => run_launcher(ctx),
    }
}

fn hook_event_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("hook"), Some(event), None) => Some(event),
        _ => None,
    }
}

fn run_hook_command(event: &str) {
    use std::io::{Read, Write};

    let mut payload = Vec::new();
    if let Err(err) = std::io::stdin().read_to_end(&mut payload) {
        eprintln!("oxplow hook failed to read stdin: {err}");
        return;
    }
    if payload.is_empty() {
        payload.extend_from_slice(b"{}");
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("oxplow hook failed to start runtime: {err}");
            return;
        }
    };

    let result = runtime.block_on(forward_hook(event, payload));
    match result {
        Ok(body) if !body.is_empty() => {
            if let Err(err) = std::io::stdout().write_all(&body) {
                eprintln!("oxplow hook failed to write response: {err}");
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("oxplow hook forwarding failed: {err}"),
    }
}

async fn forward_hook(event: &str, payload: Vec<u8>) -> Result<Vec<u8>, reqwest::Error> {
    let base_url = std::env::var("OXPLOW_HOOK_BASE_URL").unwrap_or_default();
    let url = format!("{}/{}", base_url.trim_end_matches('/'), event);
    let token = std::env::var("OXPLOW_HOOK_TOKEN").unwrap_or_default();

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header(
            "X-Oxplow-Stream",
            std::env::var("OXPLOW_STREAM_ID").unwrap_or_default(),
        )
        .header(
            "X-Oxplow-Thread",
            std::env::var("OXPLOW_THREAD_ID").unwrap_or_default(),
        )
        .header(
            "X-Oxplow-Pane",
            std::env::var("OXPLOW_PANE").unwrap_or_default(),
        )
        .body(payload)
        .send()
        .await?;

    Ok(response.bytes().await?.to_vec())
}

/// Reopen the project windows recorded in the global session (the set
/// open at last exit). Spawns one process per still-valid project dir
/// and returns whether at least one was reopened. Entries whose dir is
/// gone or no longer an Oxplow project are skipped.
fn restore_session() -> bool {
    let Some(session) = session_store() else {
        return false;
    };
    let mut spawned = 0;
    for path in session.list() {
        let dir = std::path::Path::new(&path);
        if !dir.join(".oxplow").is_dir() {
            continue; // gone or never initialized — don't restore
        }
        match oxplow_app::spawn_project_window(dir, true) {
            Ok(()) => {
                spawned += 1;
                tracing::info!(project = %path, "restored session window");
            }
            Err(e) => {
                tracing::warn!(error = %e, project = %path, "failed to restore session window")
            }
        }
    }
    spawned > 0
}

/// The global session store (`session.json` in the app-config dir),
/// or `None` if the config dir is undiscoverable.
fn session_store() -> Option<oxplow_config::SessionProjects> {
    oxplow_config::global_config_dir()
        .map(|d| oxplow_config::SessionProjects::new(d.join("session.json")))
}

/// The set of project dirs with a currently-live window: `self_dir`
/// (which holds its own instance lock during boot) plus every recent
/// project whose `.oxplow/instance.lock` is held by a live process.
/// Used to re-snapshot the session on a fresh boot so closed/stale
/// projects drop off and don't accumulate across runs.
fn live_project_dirs(self_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let self_canon = std::fs::canonicalize(self_dir).unwrap_or_else(|_| self_dir.to_path_buf());
    seen.insert(self_canon.clone());
    out.push(self_canon);
    if let Some(cfg) = oxplow_config::global_config_dir() {
        let recents = oxplow_config::RecentProjects::new(cfg.join("recent-projects.json"));
        for r in recents.list() {
            let p = std::path::PathBuf::from(&r.path);
            let canon = std::fs::canonicalize(&p).unwrap_or(p);
            if !seen.insert(canon.clone()) {
                continue;
            }
            if oxplow_app::is_project_locked(&canon) {
                out.push(canon);
            }
        }
    }
    out
}

/// Whether any oxplow project OTHER than `self_dir` currently has a
/// live window (its instance lock is held). Used to decide whether a
/// closing window is "one of several" (drop it from the restore set)
/// vs. the last/only window (keep it to restore next launch).
fn other_window_alive(self_dir: &std::path::Path) -> bool {
    let self_canon = std::fs::canonicalize(self_dir).ok();
    let Some(cfg) = oxplow_config::global_config_dir() else {
        return false;
    };
    let recents = oxplow_config::RecentProjects::new(cfg.join("recent-projects.json"));
    recents.list().into_iter().any(|r| {
        let canon = std::fs::canonicalize(&r.path).ok();
        if canon.is_some() && canon == self_canon {
            return false; // skip self
        }
        oxplow_app::is_project_locked(std::path::Path::new(&r.path))
    })
}

/// Publish a loopback focus channel for this project and serve it on a
/// background thread: a nonce-matching ping raises/focuses the main
/// window. Lets a second `open_project` of the same dir focus the
/// existing window (see `oxplow_app::request_focus`) instead of being
/// turned away by the instance lock. Best-effort — any failure just
/// means the second open falls back to the "already open" error.
fn start_focus_listener(app: &tauri::AppHandle, project_dir: std::path::PathBuf) {
    use std::io::{BufRead, BufReader};
    use tauri::Manager;

    let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "focus listener bind failed");
            return;
        }
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return;
    };
    let nonce = oxplow_app::new_focus_nonce();
    let layout = AppLayout::for_project(&project_dir);
    if let Err(e) = oxplow_app::write_instance_info(
        &layout,
        &oxplow_app::InstanceInfo {
            focus_port: port,
            nonce: nonce.clone(),
        },
    ) {
        tracing::warn!(error = %e, "failed to publish focus channel");
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() && line.trim() == nonce {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.unminimize();
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        }
    });
}

/// Resolve the project dir for this process:
///   1. first positional CLI arg (`oxplow <dir>`),
///   2. `OXPLOW_PROJECT_DIR` (set by the dev script and by the
///      `open_project` spawn),
///   3. otherwise `None` → launcher mode.
///
/// The cwd fallback was intentionally dropped: a bare launch shows the
/// launcher rather than silently adopting whatever directory it was
/// started from.
fn resolve_project_dir() -> Option<std::path::PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        // Skip flag-like args (e.g. macOS may pass `-psn_…`).
        if !arg.starts_with('-') {
            return Some(std::path::PathBuf::from(arg));
        }
    }
    std::env::var_os("OXPLOW_PROJECT_DIR").map(std::path::PathBuf::from)
}

/// Resolve the global recent-projects store, optionally record the
/// just-opened project, and manage it on the app. Shared by both
/// launch modes so the launcher screen and a project window can each
/// list / open / forget recent projects.
fn install_recent_projects(app: &tauri::AppHandle, record: Option<std::path::PathBuf>) {
    use tauri::Manager;
    let cfg_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let store: oxplow_tauri_ipc::RecentProjectsState = Arc::new(
        oxplow_config::RecentProjects::new(cfg_dir.join("recent-projects.json")),
    );
    if let Some(dir) = record {
        store.record(&dir);
    }
    app.manage(store);
}

/// Launcher mode — no project, no `Services`. Hosts only the
/// recent-projects surface (`commands::launch`) and the launcher
/// window; the renderer's `<Root>` sees `mode: "launcher"` and renders
/// the start screen.
fn run_launcher(ctx: tauri::Context) {
    tracing::info!("booting in launcher mode (no project dir)");
    let specta = specta_builder();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .manage(oxplow_tauri_ipc::LaunchInfo::launcher())
        .setup(move |app| {
            specta.mount_events(app);
            install_recent_projects(app.handle(), None);
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .run(ctx)
        .expect("error while running tauri application");
}

/// Setup mode — a directory was opened that isn't an Oxplow project
/// yet (no `.oxplow/`). Like the launcher (no `Services`), but the
/// renderer shows the "Create an Oxplow project here?" screen for this
/// dir. Confirming calls `setup_project` (creates `.oxplow/` and
/// relaunches into `run_project`); declining calls `abort_setup`
/// (exits). Nothing is recorded into recents until setup is confirmed.
fn run_setup(project_dir: std::path::PathBuf, ctx: tauri::Context) {
    tracing::info!(project = %project_dir.display(), "booting in setup mode (no .oxplow yet)");
    let specta = specta_builder();
    let launch_info = oxplow_tauri_ipc::LaunchInfo::setup(project_dir.to_string_lossy());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .manage(launch_info)
        .setup(move |app| {
            specta.mount_events(app);
            install_recent_projects(app.handle(), None);
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .run(ctx)
        .expect("error while running tauri application");
}

/// Project mode — boot `Services` for `project_dir` and run the full
/// app shell, exactly as before the launcher existed.
fn run_project(project_dir: std::path::PathBuf, ctx: tauri::Context) {
    let layout = AppLayout::for_project(&project_dir);

    // Per-project single-instance guard. Two processes on the same
    // `.oxplow/state.sqlite` would double the fs/git watchers and
    // contend on SQLite's writer lock, so refuse the second boot. Held
    // for the life of the process (leaked); the OS frees it on exit.
    match oxplow_app::try_acquire_instance_lock(&layout) {
        Ok(Some(lock)) => {
            Box::leak(Box::new(lock));
        }
        Ok(None) => {
            tracing::error!(
                project = %layout.project_dir.display(),
                "project already open in another oxplow window; exiting"
            );
            eprintln!(
                "oxplow: this project is already open in another window: {}",
                layout.project_dir.display()
            );
            std::process::exit(0);
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to acquire instance lock; continuing without guard");
        }
    }

    // Record this window in the global session so a later bare launch
    // reopens it. A window closing does NOT remove its entry — closing
    // the (last) window is how the user "exits", and they expect it
    // restored. Instead we re-snapshot the live set on each fresh boot:
    //   - restore spawns carry OXPLOW_RESTORING → just add self, so
    //     concurrent restores don't clobber the set being restored;
    //   - a user-initiated open re-snapshots the live window set
    //     (self + any project whose instance lock is still held), which
    //     drops projects closed in a prior run so they don't accumulate.
    if let Some(session) = session_store() {
        if std::env::var_os("OXPLOW_RESTORING").is_some() {
            session.add(&project_dir);
        } else {
            session.replace(&live_project_dirs(&project_dir));
        }
    }

    // Services::boot synchronously calls `tokio::spawn` (PtyManager owner
    // task), which requires an entered Tokio runtime. Tauri builds its
    // own runtime later, so we stand up a dedicated multi-thread runtime
    // here, enter it for the duration of boot, and leak the runtime so
    // background tasks keep running for the life of the process.
    let boot_runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime"),
    ));
    let _enter = boot_runtime.enter();
    let services = Services::boot(layout).expect("services boot");

    let state: AppState = Arc::new(services);
    let event_bus = state.events.clone();
    let lsp_clients = state.lsp_clients.clone();
    let terminal_sessions = state.terminal_sessions.clone();

    // Run daemon recovery — close any agent_turn rows that the
    // previous boot left open, reset agent_status rows from
    // Running/AwaitingUser to Stopped. Synchronous so the renderer
    // doesn't see stale state.
    let recovery = state.recovery.clone();
    tauri::async_runtime::block_on(async move {
        if let Err(e) = recovery.run().await {
            tracing::warn!(error = %e, "daemon recovery failed");
        }
    });

    // Ensure the project's primary stream (and its default thread)
    // exist. `StreamService::ensure_primary` itself seeds the
    // auto-generated thread, so a single call covers both invariants
    // — every stream owns ≥1 thread.
    let streams = state.streams.clone();
    boot_runtime.block_on(async move {
        match streams.ensure_primary().await {
            Ok(s) => tracing::info!(stream_id = %s.id, "primary stream ready"),
            Err(e) => tracing::warn!(error = %e, "ensure_primary failed at boot"),
        }
    });

    // Start the file-snapshot manager's watcher loop for every
    // registered stream. The services themselves are constructed
    // inside Services::boot so TaskService can request snapshots on
    // `in_progress` transitions; here we wire the fs-watch listeners
    // + the boot-time sweep + cleanup on the primary stream.
    state.snapshot_captures.spawn_all_watchers();
    // GitRefsChanged listeners need an event-bus subscriber per
    // service so a commit in any worktree re-stamps that stream's
    // latest snapshot. The primary's listener also covers the legacy
    // wiki-watcher wiring below.
    for svc in state.snapshot_captures.list() {
        svc.spawn_git_refs_listener();
    }
    // The startup sweep + cleanup loop below operate on the primary
    // stream's service. Each per-stream worktree has its own service
    // via the registry; only the primary needs the sweep at boot.
    let snapshot_svc = state
        .snapshot_captures
        .primary()
        .expect("primary snapshot capture registered at boot");
    let project_dir = state.layout.project_dir.clone();
    // Startup sweep: any file whose current content doesn't match
    // the latest snapshot row (or was never snapshotted) gets
    // queued + captured now. Backfills changes that landed while
    // the daemon wasn't running. Spawned off the boot path because
    // hashing a large worktree can take a few seconds.
    {
        let svc = snapshot_svc.clone();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::Snapshot,
            label: "Scanning worktree for snapshot changes".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        boot_runtime.spawn(async move {
            let hud_started = std::time::Instant::now();
            match svc.enqueue_startup_diff().await {
                Ok(0) => {
                    tracing::info!(
                        elapsed_ms = hud_started.elapsed().as_millis() as u64,
                        "startup snapshot HUD: nothing to capture",
                    );
                    bts.complete(&task_id, Some(serde_json::json!({"captured": 0})));
                }
                Ok(n) => {
                    tracing::info!(queued = n, "startup snapshot sweep: queued files");
                    bts.update(
                        &task_id,
                        UpdateInput {
                            label: Some(format!("Capturing {n} changed files")),
                            ..Default::default()
                        },
                    );
                    match svc
                        .request_snapshot(oxplow_app::events::SnapshotSourceKind::Startup)
                        .await
                    {
                        Ok(parent) => {
                            tracing::info!(
                                snapshot_id = ?parent,
                                queued = n,
                                elapsed_ms = hud_started.elapsed().as_millis() as u64,
                                "startup snapshot HUD: complete",
                            );
                            bts.complete(&task_id, Some(serde_json::json!({"snapshotId": parent})))
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "startup snapshot sweep: capture failed");
                            bts.fail(&task_id, e.to_string(), None);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "startup snapshot sweep: walk failed");
                    bts.fail(&task_id, e.to_string(), None);
                }
            }
        });
    }
    // Snapshot cleanup loop — prunes rows older than the configured
    // retention window (keeping the most-recent row per path) and
    // GC's orphaned blob files. Runs ~60s after boot and every 24h.
    {
        let retention_days = state
            .config
            .read()
            .map(|c| c.snapshot_retention_days)
            .unwrap_or(7);
        snapshot_svc.spawn_cleanup_loop(retention_days, Some(state.background_tasks.clone()));
    }

    // Comment cleanup loop — prunes resolved/orphaned comment threads
    // whose last activity is older than the retention window. Runs at
    // boot and every 24h.
    {
        use oxplow_domain::stores::CommentStore;
        const COMMENT_RETENTION_DAYS: i64 = 14;
        let comment_store = state.comment_store.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = comment_store.cleanup(COMMENT_RETENTION_DAYS).await {
                    tracing::warn!("comment cleanup failed: {e}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
            }
        });
    }

    // Per-stream fs + .git/refs watchers — bridges file changes onto
    // the EventBus so the renderer's QuickOpen, project panel, history,
    // git dashboard, etc. refresh without polling. Held in a registry
    // for the life of the daemon; dropping it cancels every watcher.
    //
    // Pushed off the synchronous boot path: registering recursive
    // watches over a large worktree can take a moment to settle.
    // Surfacing it as a BackgroundTask lets the renderer paint while
    // watchers come up.
    {
        let stream_service = state.streams.clone();
        let watch_bus = event_bus.clone();
        let watch_project_dir = project_dir.clone();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::Git,
            label: "Starting workspace watchers".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        boot_runtime.spawn(async move {
            let registry = oxplow_app::workspace_watch::WorkspaceWatchRegistry::spawn(
                stream_service,
                watch_bus,
                watch_project_dir,
            )
            .await;
            Box::leak(Box::new(registry));
            bts.complete(&task_id, None);
        });
    }

    // Wiki notes watcher: keeps `wiki_page` rows in sync with
    // `.oxplow/wiki/<slug>.md` on disk (initial scan + debounced
    // re-syncs on change). Held alive for the life of the process.
    //
    // One-shot legacy migration runs synchronously before the watcher
    // spawns: earlier versions stored bodies under `.oxplow/notes/`,
    // and the rename to `.oxplow/wiki/` would otherwise leave
    // existing pages stranded.
    oxplow_app::wiki_pages::migrate_legacy_notes_dir(&state.layout.project_dir);
    {
        let wiki_store = state.wiki_page_store.clone();
        let wiki_page_refs = state.page_ref_store.clone();
        let wiki_dir = state.layout.project_dir.clone();
        let wiki_events = event_bus.clone();
        // Wiki is project-wide; pin to the primary stream's service.
        // Tracked for migration to a dedicated wiki pseudo-stream
        // under epic #28's follow-up.
        let wiki_snapshot_capture = state.snapshot_captures.primary();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::NotesResync,
            label: "Initial wiki notes scan".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        boot_runtime.spawn(async move {
            if let Some(watcher) = oxplow_app::wiki_pages_watch::WikiPagesWatcher::spawn(
                wiki_dir,
                wiki_store,
                wiki_page_refs,
                wiki_events,
                wiki_snapshot_capture,
            )
            .await
            {
                Box::leak(Box::new(watcher));
            }
            bts.complete(&task_id, None);
        });
    }

    // Config watcher: hot-reload `oxplow.yaml` on out-of-band edits
    // (e.g. the agent running `/oxplow:configure`, which Writes a
    // `collection:` block) so config changes go live without a restart.
    // Held alive for the life of the process.
    {
        let cfg_services = state.clone();
        boot_runtime.spawn(async move {
            if let Some(watcher) = oxplow_app::config_watch::ConfigWatcher::spawn(cfg_services) {
                Box::leak(Box::new(watcher));
            }
        });
    }

    // Lightweight self-diagnostics: once a minute, log RSS + open
    // fds + stream count so a long-running process leaves a trail
    // we can correlate against system-wide weirdness. See
    // `crates/oxplow-app/src/diagnostics.rs`.
    {
        let streams = state.stream_store.clone();
        boot_runtime.spawn(async move {
            oxplow_app::diagnostics::spawn(streams);
        });
    }

    // Unified page-ref graph backfill: re-project every existing
    // task, link, effort, and finding into the `page_ref`
    // table. Idempotent — second-run touches the same rows. Wiki
    // bodies and recent commits are covered by their own watchers
    // (above + below).
    {
        let page_refs = state.page_ref_store.clone();
        let tasks = state.task_store.clone();
        let links = state.task_link_store.clone();
        let efforts = state.effort_store.clone();
        let findings = state.code_quality_store.clone();
        let notes = state.work_note_store.clone();
        boot_runtime.spawn(async move {
            let counts = oxplow_app::page_ref_backfill::run(
                page_refs, tasks, links, efforts, findings, notes,
            )
            .await;
            tracing::info!(?counts, "page-ref backfill done");
        });
    }

    // Commit indexer: walk the most-recent N commits at boot to
    // populate `(git-commit:<sha>) -> (file/task/wiki/finding)`
    // edges, then re-scan whenever git refs change. Idempotent —
    // already-indexed commits are skipped via a one-row probe.
    {
        let repo_path = state.layout.project_dir.clone();
        let page_refs = state.page_ref_store.clone();
        let mut rx = state.events.subscribe();
        boot_runtime.spawn(async move {
            let n = oxplow_app::commit_indexer::index_recent(
                &repo_path,
                &page_refs,
                oxplow_app::commit_indexer::DEFAULT_INDEX_DEPTH,
            )
            .await;
            tracing::info!(indexed = n, "commit indexer initial scan done");
            // Re-index on every refs change. The watcher debounces;
            // the indexer's own existence-probe makes the scan cheap
            // when nothing's new.
            loop {
                match rx.recv().await {
                    Ok(oxplow_app::events::OxplowEvent::GitRefsChanged { .. }) => {
                        let _ = oxplow_app::commit_indexer::index_recent(
                            &repo_path,
                            &page_refs,
                            oxplow_app::commit_indexer::DEFAULT_INDEX_DEPTH,
                        )
                        .await;
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // Search indexer: backfill the unified FTS index from current state,
    // then keep it fresh off the event bus (tasks/comments/notes/wiki +
    // per-stream file snapshots). One detached background task.
    {
        let indexer = oxplow_app::indexer::Indexer::new(state.clone());
        let rx = state.events.subscribe();
        boot_runtime.spawn(async move {
            indexer.run(rx).await;
        });
    }

    // Boot the in-process control plane (axum server hosting the
    // plugin's HTTP hook receiver + the streamable-HTTP MCP transport).
    // The handle's URLs + token feed the per-spawn plugin writer in
    // terminal.rs; nothing here needs to keep the handle around — the
    // axum task is detached.
    let control_plane = boot_runtime
        .block_on(async { oxplow_control_plane::spawn(state.clone()).await })
        .expect("control plane boot");
    let plugin_runtime: PluginRuntimeState = Arc::new(PluginRuntime {
        hook_base_url: control_plane.hook_base_url(),
        mcp_endpoint_url: control_plane.mcp_endpoint_url(),
        hook_token: control_plane.hook_token.clone(),
    });

    let specta = specta_builder();
    let launch_info = oxplow_tauri_ipc::LaunchInfo::project(project_dir.to_string_lossy());
    let project_dir_for_setup = project_dir.clone();
    let project_dir_for_close = project_dir.clone();
    let project_dir_for_focus = project_dir.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .on_window_event(move |_window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                // Closing one window while OTHER windows are still open
                // means "I'm done with this project" → drop it from the
                // restore set. Closing the last/only window, or quitting
                // the whole app (Cmd-Q → ExitRequested sets QUITTING),
                // preserves the set so it's restored next launch.
                let quitting = QUITTING.load(std::sync::atomic::Ordering::SeqCst);
                if !quitting && other_window_alive(&project_dir_for_close) {
                    if let Some(session) = session_store() {
                        session.remove(&project_dir_for_close);
                    }
                }
                // The instance lock releases on process death, so a
                // stale focus port is never used; clear it anyway.
                oxplow_app::clear_instance_info(&project_dir_for_close);
            }
        })
        .setup(move |app| {
            specta.mount_events(app);
            // Record this project into the global recents so the
            // launcher offers it next time, and expose the store to
            // the in-window "Open Recent" surface.
            install_recent_projects(app.handle(), Some(project_dir_for_setup));
            // Publish a focus channel so a second open of this project
            // raises this window instead of failing on the lock.
            start_focus_listener(app.handle(), project_dir_for_focus.clone());
            spawn_event_bridge(app.handle().clone(), event_bus.clone());
            spawn_lsp_event_bridge(app.handle().clone(), lsp_clients.clone());
            spawn_terminal_event_bridge(app.handle().clone(), terminal_sessions.clone());
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .manage(state)
        .manage(plugin_runtime)
        .manage(launch_info)
        .build(ctx)
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // A full app quit (Cmd-Q, OS shutdown) flips QUITTING so the
            // per-window CloseRequested handlers preserve the session set
            // instead of dropping each window as it tears down.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
}

/// Forwards every `OxplowEvent` from the in-process bus onto the
/// Tauri event channel so the renderer can `listen("oxplow:event", …)`.
/// Lagging subscribers (slow renderer) are tolerated — they'll see a
/// `RecvError::Lagged` and the renderer typically refetches on
/// reconnect rather than replaying every event.
fn spawn_event_bridge(app: tauri::AppHandle, bus: oxplow_app::EventBus) {
    tauri::async_runtime::spawn(async move {
        let mut rx = bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit(OXPLOW_EVENT_CHANNEL, &event) {
                        tracing::warn!(?err, "failed to emit oxplow event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `LspBridgeEvent` from the LSP client registry onto
/// `lsp:event` for the renderer's lsp.ts module.
fn spawn_lsp_event_bridge(
    app: tauri::AppHandle,
    registry: oxplow_app::lsp_clients::LspClientRegistry,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = registry.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit("lsp:event", &event) {
                        tracing::warn!(?err, "failed to emit lsp event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "lsp event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `TerminalBridgeEvent` from the terminal session
/// registry onto `terminal:event` for the renderer's TerminalPane.
fn spawn_terminal_event_bridge(
    app: tauri::AppHandle,
    registry: oxplow_app::terminal_sessions::TerminalSessionRegistry,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = registry.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit("terminal:event", &event) {
                        tracing::warn!(?err, "failed to emit terminal event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "terminal event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,oxplow_=debug"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
