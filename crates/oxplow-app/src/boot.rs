//! Shared post-`Services::boot` orchestration.
//!
//! Everything that must happen between constructing [`Services`] and
//! serving requests — recovery, primary-stream seeding, and the fleet
//! of background watchers/indexers — lives here so the two hosts (the
//! Tauri desktop shell and the headless `oxplow-daemon`) run the exact
//! same boot path instead of drifting copies.
//!
//! Must be awaited from inside a Tokio runtime; every long-lived
//! watcher is `tokio::spawn`ed and detached (registries are
//! intentionally leaked — they live for the life of the process).

use std::sync::Arc;

use crate::background_task::{StartInput, UpdateInput};
use crate::{BackgroundTaskKind, Services};

/// Run recovery + seed the primary stream, then spawn the standard
/// background tasks (snapshot watchers + startup sweep + cleanup,
/// comment cleanup, workspace + wiki + config watchers, diagnostics,
/// page-ref backfill, commit indexer, search indexer).
///
/// The two awaited steps run synchronously on purpose: the first
/// client must not observe pre-recovery agent state or a project with
/// no primary stream.
pub async fn run_boot_orchestration(state: &Arc<Services>) {
    let event_bus = state.events.clone();

    // Daemon recovery — close any agent_turn rows that the previous
    // boot left open, reset agent_status rows from Running/AwaitingUser
    // to Stopped. Synchronous so clients don't see stale state.
    if let Err(e) = state.recovery.run().await {
        tracing::warn!(error = %e, "daemon recovery failed");
    }

    // Ensure the project's primary stream (and its default thread)
    // exist. `StreamService::ensure_primary` itself seeds the
    // auto-generated thread, so a single call covers both invariants
    // — every stream owns ≥1 thread.
    match state.streams.ensure_primary().await {
        Ok(s) => tracing::info!(stream_id = %s.id, "primary stream ready"),
        Err(e) => tracing::warn!(error = %e, "ensure_primary failed at boot"),
    }

    // Start the file-snapshot manager's watcher loop for every
    // registered stream, plus per-stream GitRefsChanged listeners so a
    // commit in any worktree re-stamps that stream's latest snapshot.
    state.snapshot_captures.spawn_all_watchers();
    for svc in state.snapshot_captures.list() {
        svc.spawn_git_refs_listener();
    }

    // Startup sweep + cleanup loop operate on the primary stream's
    // service. Each per-stream worktree has its own service via the
    // registry; only the primary needs the sweep at boot.
    let snapshot_svc = state
        .snapshot_captures
        .primary()
        .expect("primary snapshot capture registered at boot");

    // Hold the effort-start gate closed until the initial sweep below
    // completes, so an agent dispatched during the sweep can't open an
    // effort whose start snapshot reflects a half-captured tree. Set
    // synchronously here (before the spawn) so the gate is up the
    // instant boot returns.
    snapshot_svc.begin_initial_sweep();

    // Startup sweep: any file whose current content doesn't match the
    // latest snapshot row (or was never snapshotted) gets queued +
    // captured now. Backfills changes that landed while the daemon
    // wasn't running. Spawned off the boot path because hashing a
    // large worktree can take a few seconds.
    {
        let svc = snapshot_svc.clone();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::Snapshot,
            label: "Scanning worktree for snapshot changes".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        tokio::spawn(async move {
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
                        .request_snapshot(crate::events::SnapshotSourceKind::Startup)
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
            // Release the effort-start gate on every path — including
            // failure. A failed sweep leaves a best-effort baseline;
            // blocking efforts forever would be worse than a partial one.
            svc.mark_initial_complete();
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
    // the EventBus so clients refresh without polling. Held in a
    // registry for the life of the daemon. Pushed off the synchronous
    // boot path: registering recursive watches over a large worktree
    // can take a moment to settle.
    {
        let stream_service = state.streams.clone();
        let watch_bus = event_bus.clone();
        let watch_project_dir = state.layout.project_dir.clone();
        let watch_filter = {
            let cfg = crate::config_service::read_config(&state.config);
            oxplow_fs_watch::WorkspaceFilter::for_project(
                &watch_project_dir,
                &cfg.generated.exclude,
                &cfg.generated.include,
            )
        };
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::Git,
            label: "Starting workspace watchers".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        tokio::spawn(async move {
            let registry = crate::workspace_watch::WorkspaceWatchRegistry::spawn(
                stream_service,
                watch_bus,
                watch_project_dir,
                watch_filter,
            )
            .await;
            Box::leak(Box::new(registry));
            bts.complete(&task_id, None);
        });
    }

    // Wiki notes watcher: keeps `wiki_page` rows in sync with
    // `.oxplow/wiki/<slug>.md` on disk (initial scan + debounced
    // re-syncs on change). One-shot legacy migration runs
    // synchronously before the watcher spawns.
    crate::wiki_pages::migrate_legacy_notes_dir(&state.layout.project_dir);
    {
        let wiki_store = state.wiki_page_store.clone();
        let wiki_page_refs = state.page_ref_store.clone();
        let wiki_dir = state.layout.project_dir.clone();
        let wiki_events = event_bus.clone();
        // Wiki is project-wide; pin to the primary stream's service.
        let wiki_snapshot_capture = state.snapshot_captures.primary();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::NotesResync,
            label: "Initial wiki notes scan".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        tokio::spawn(async move {
            if let Some(watcher) = crate::wiki_pages_watch::WikiPagesWatcher::spawn(
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

    // Config watcher: hot-reload `.oxplow/project.yaml` on out-of-band edits so
    // config changes go live without a restart.
    {
        let cfg_services = state.clone();
        tokio::spawn(async move {
            if let Some(watcher) = crate::config_watch::ConfigWatcher::spawn(cfg_services) {
                Box::leak(Box::new(watcher));
            }
        });
    }

    // Agent stall watchdog: once a minute, re-derive every thread's
    // status against the wall clock. Catches agent processes that died
    // mid-turn without emitting a Stop hook (API errors) — flips the
    // stuck Working dot to Stalled and alerts when in_progress work
    // sits on a non-running agent. See agent_stall_watch.rs.
    crate::agent_stall_watch::AgentStallWatch::new(
        state.agent_status_store.clone(),
        state.hook_event_store.clone(),
        state.task_store.clone(),
        state.output_activity.clone(),
        event_bus.clone(),
    )
    .spawn();

    // Lightweight self-diagnostics: once a minute, log RSS + open fds
    // + stream count so a long-running process leaves a trail.
    {
        let streams = state.stream_store.clone();
        tokio::spawn(async move {
            crate::diagnostics::spawn(streams);
        });
    }

    // Unified page-ref graph backfill: re-project every existing task,
    // link, effort, and finding into the `page_ref` table. Idempotent.
    {
        let page_refs = state.page_ref_store.clone();
        let tasks = state.task_store.clone();
        let links = state.task_link_store.clone();
        let efforts = state.effort_store.clone();
        let findings = state.code_quality_store.clone();
        let notes = state.work_note_store.clone();
        tokio::spawn(async move {
            let counts =
                crate::page_ref_backfill::run(page_refs, tasks, links, efforts, findings, notes)
                    .await;
            tracing::info!(?counts, "page-ref backfill done");
        });
    }

    // Commit indexer: walk the most-recent N commits at boot, then
    // re-scan whenever git refs change. Idempotent.
    {
        let repo_path = state.layout.project_dir.clone();
        let page_refs = state.page_ref_store.clone();
        let mut rx = state.events.subscribe();
        tokio::spawn(async move {
            let n = crate::commit_indexer::index_recent(
                &repo_path,
                &page_refs,
                crate::commit_indexer::DEFAULT_INDEX_DEPTH,
            )
            .await;
            tracing::info!(indexed = n, "commit indexer initial scan done");
            loop {
                match rx.recv().await {
                    Ok(crate::events::OxplowEvent::GitRefsChanged { .. }) => {
                        let _ = crate::commit_indexer::index_recent(
                            &repo_path,
                            &page_refs,
                            crate::commit_indexer::DEFAULT_INDEX_DEPTH,
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

    // Search indexer: backfill the unified FTS index from current
    // state, then keep it fresh off the event bus.
    {
        let indexer = crate::indexer::Indexer::new(state.clone());
        let rx = state.events.subscribe();
        tokio::spawn(async move {
            indexer.run(rx).await;
        });
    }

    // Metric runner (tsk213, P3): seed config-declared metric definitions,
    // then run on-snapshot gauges as snapshots land + reseed on config change.
    {
        let metrics = state.metrics.clone();
        let rx = state.events.subscribe();
        tokio::spawn(async move {
            metrics.run(rx).await;
        });
    }

    // Metric aggregate cube (tsk96): fold each partial-scope measure's captures
    // into `metric_cube` as facts land, so a sparkline is a GROUP BY over a few
    // hundred pre-folded rows instead of a replay over every fact. Backfills once,
    // then keeps up off `MetricSamplesChanged`.
    //
    // Purely an accelerator — if this task never ran, every read would take the
    // fact path exactly as it did before the cube existed.
    {
        let builder = crate::metric_cube::MetricCubeBuilder::new((*state.fact_store).clone())
            .with_visibility(state.metric_visibility.clone());
        let rx = state.events.subscribe();
        tokio::spawn(async move {
            crate::metric_cube::run(builder, rx).await;
        });
    }

    // Tree-metric BASELINE (tsk41). A `per-path` measure folds over each capture's
    // snapshot file rows, so a repo-wide total needs ONE snapshot listing the whole
    // tree. On a fresh project — or after the V54 wipe — there isn't one, and delta
    // snapshots alone never get there (a file only enters the fold once some commit
    // touches it). Capture a full tree once; the on-snapshot gauges then run over
    // every path via the normal event path. No-op once the fold has facts, so this
    // costs nothing on a warm boot.
    {
        let state = state.clone();
        tokio::spawn(async move {
            match state.rebuild_metric_baseline(false).await {
                Ok(r) if r.ran => tracing::info!(
                    gauges = r.gauges_run,
                    failed = ?r.failed,
                    "metric tree baseline: complete",
                ),
                Ok(_) => tracing::debug!("metric tree baseline: nothing to do"),
                Err(e) => tracing::warn!(error = %e, "metric tree baseline failed"),
            }
        });
    }
}
