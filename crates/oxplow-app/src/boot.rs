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
    // registered stream, plus per-stream VcsRefsChanged listeners so a
    // commit in any worktree re-stamps that stream's latest snapshot.
    state.snapshot_captures.spawn_all_watchers();
    for svc in state.snapshot_captures.list() {
        svc.spawn_git_refs_listener(&state.ref_moves);
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
                        .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
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

    // Snapshot cleanup loop — expires blob CONTENT older than the configured
    // retention window (tsk105: the rows are permanent records; each
    // (stream, path)'s newest content is kept at any age). Runs ~60s after
    // boot and every 24h. `snapshotRetentionDays: 0` disables it.
    {
        let retention_days = state
            .config
            .read()
            .map(|c| c.snapshot_retention_days)
            .unwrap_or(7);
        snapshot_svc.spawn_cleanup_loop(retention_days, Some(state.background_tasks.clone()));
    }

    // The event pump: delivers the log to its consumers (page_ref
    // projections first); catches up on anything logged while down.
    crate::effort_reactors::register(state);
    crate::indexer::register(state);
    crate::symbol_collector::register(state);
    // A stream's working tree and open efforts, re-analyzed as it moves (P7.B4).
    crate::change_reactor::register(state);
    // A disabled plugin contribution's repair work item (P7.C2).
    crate::plugin_repair::register(state);
    // `on:` collectors (P7.B3), after the consumers they may name.
    crate::collector_triggers::register(state);
    // Config changes reach the extension catalog, the provider registry and
    // the metric catalog (P7.B6).
    crate::config_reactors::register(state);
    // A collector that registered a new entity may let a model compile.
    crate::extension_models::register(state);
    state.event_pump.clone().spawn();
    // `every:` collectors: the scheduler runs `collector.sync` as the system.
    crate::collector_runner::spawn_scheduler(state.clone());
    // Core's capability providers, before the registry publishes the
    // external ones it starts.
    if let Err(e) = crate::capabilities::publish_core(state).await {
        tracing::warn!(error = %e, "publishing the core capability providers failed");
    }
    crate::providers::registry::spawn_reconciler(state.clone());
    crate::providers::sync::spawn_sync_scheduler(state.clone());
    crate::extension_commands::spawn_reconciler(state.clone());
    // Open efforts' evidence, an asset over the tables it reads (P7.B6).
    crate::effort_evidence::register(state);

    // Metric retention loop (tsk93) — OPT-IN: `metricRetentionDays` defaults
    // to 0 = keep everything (per-test history is what makes the substrate
    // worth having). When enabled, a daily pass prunes captures older than
    // the window that no current value stands on — effort-stamped captures,
    // each producer's newest, and fold-live facts' captures are always kept
    // (see `prune_aged_captures`). Config is re-read each pass, so flipping
    // the setting takes effect without a restart.
    {
        let facts = state.fact_store.clone();
        let config = state.config.clone();
        tokio::spawn(async move {
            // Stay out of boot's way (backfill, sweeps, snapshot cleanup).
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            loop {
                let days = config.read().map(|c| c.metric_retention_days).unwrap_or(0);
                if days > 0 {
                    let cutoff = oxplow_domain::Timestamp::from_unix_ms(
                        oxplow_domain::Timestamp::now().unix_ms() - (days as i64) * 86_400_000,
                    );
                    match facts.prune_aged_captures(cutoff).await {
                        Ok(n) if n > 0 => {
                            tracing::info!(pruned = n, days, "metric retention pass")
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "metric retention pass failed"),
                    }
                }
                // Detail compaction (tsk211) runs INDEPENDENTLY of the prune
                // above: it is on by default, because nulling a drill-in payload
                // can't change a metric value, whereas the prune deletes facts
                // and stays opt-in. `detail_json` was 200 MB of a 795 MB DB here
                // after three weeks — ~0.5 MB per coverage run.
                let (keep, detail_days) = config
                    .read()
                    .map(|c| {
                        (
                            c.metric_detail_max_per_producer,
                            c.metric_detail_retention_days,
                        )
                    })
                    .unwrap_or((0, 0));
                let detail_cutoff = (detail_days > 0).then(|| {
                    oxplow_domain::Timestamp::from_unix_ms(
                        oxplow_domain::Timestamp::now().unix_ms()
                            - (detail_days as i64) * 86_400_000,
                    )
                });
                match facts
                    .compact_capture_details(detail_cutoff, (keep > 0).then_some(keep))
                    .await
                {
                    Ok(n) if n > 0 => tracing::info!(
                        compacted = n,
                        keep_per_producer = keep,
                        days = detail_days,
                        "metric detail compaction pass"
                    ),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "metric detail compaction failed"),
                }
                // Return the WAL's high-water mark to the OS (tsk216). The
                // automatic checkpoint is PASSIVE — it restarts the WAL in place
                // and reuses the space — so after a big write burst (a metric
                // rebuild took it to 169MB here) the file parks at that size
                // forever. Best-effort: TRUNCATE needs readers to have moved on,
                // and a busy result just means we retry tomorrow.
                if let Err(e) = facts.checkpoint_wal().await {
                    tracing::warn!(error = %e, "wal checkpoint failed");
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
            }
        });
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
        let watch_vcs = state.vcs.clone();
        let watch_bus = event_bus.clone();
        let watch_moves = state.ref_moves.clone();
        let watch_catalog = state.extension_catalog.clone();
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
            kind: BackgroundTaskKind::Vcs,
            label: "Starting workspace watchers".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        tokio::spawn(async move {
            let registry = crate::workspace_watch::WorkspaceWatchRegistry::spawn(
                stream_service,
                watch_vcs,
                watch_bus,
                watch_moves,
                watch_catalog,
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
        let wiki_db = state.db.clone();
        let wiki_vocabulary = state.vocabulary.clone();
        let wiki_dir = state.layout.project_dir.clone();
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
                wiki_db,
                wiki_vocabulary,
                wiki_store,
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
        (*state.event_log_store).clone(),
        state.task_store.clone(),
        state.output_activity.clone(),
        event_bus.clone(),
    )
    .spawn();

    // Each stream's recorded branch follows its checkout.
    state.branch_reconciler.clone().spawn();

    // The vocabulary (P8.D3): core plus what extensions declare, rebuilt
    // now and on every change.
    state
        .vocabulary_service
        .clone()
        .spawn(state.extension_catalog.changes());

    // Extensions' SQL models (P4.9): compiled now and on every change.
    state
        .extension_models
        .clone()
        .spawn(state.extension_catalog.changes());

    // The one change loop (P4.6, P7.B1, P7.B6): which models each commit
    // changed, which metric samples landed, which assets went stale, and
    // when the pump has a new event to read.
    crate::models_changed::spawn(
        state.db.clone(),
        state.model_watermarks.clone(),
        event_bus.clone(),
        state.assets.clone(),
        state.event_pump.clone(),
    );

    // Event retention (P3.11): expire old agent/test payloads and bodies
    // per `.context/target-architecture.md` §5.4 — a while after boot (the
    // first sweep after an upgrade may have a large backlog, and hooks
    // shouldn't meet it while the app is starting), then daily.
    {
        let db = state.db.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
            loop {
                match oxplow_db::event_retention::sweep(&db, oxplow_domain::Timestamp::now()).await
                {
                    Ok(report) => tracing::info!(?report, "event retention sweep done"),
                    Err(error) => tracing::warn!(%error, "event retention sweep failed"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
            }
        });
    }

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
        let vocabulary = state.vocabulary.clone();
        let page_refs = state.page_ref_store.clone();
        let tasks = state.task_store.clone();
        let links = state.task_link_store.clone();
        let efforts = state.effort_store.clone();
        let findings = state.code_quality_store.clone();
        let notes = state.work_note_store.clone();
        tokio::spawn(async move {
            let counts = crate::page_ref_backfill::run(
                vocabulary, page_refs, tasks, links, efforts, findings, notes,
            )
            .await;
            tracing::info!(?counts, "page-ref backfill done");
        });
    }

    // Commit indexer + branch refresh: walk the most-recent N commits
    // and restate the branch list at boot, then again whenever git refs
    // change. Idempotent.
    {
        let state = state.clone();
        let mut rx = state.ref_moves.subscribe();
        tokio::spawn(async move {
            let n = crate::commit_indexer::refresh(&state).await;
            tracing::info!(indexed = n, "commit indexer initial scan done");
            // Any move, a missed one included: the refresh is idempotent.
            while rx.recv().await.is_some() {
                crate::commit_indexer::refresh(&state).await;
            }
        });
    }

    // LSP diagnostics → `v_diagnostic` (live state; cleared here first).
    crate::lsp_diagnostics::spawn(state.clone());

    // Search indexer: backfill the unified FTS index from current state;
    // the `search.index` pump consumer keeps it fresh from the event log.
    {
        let indexer = crate::indexer::Indexer::new(state.clone());
        tokio::spawn(async move {
            indexer.backfill().await;
        });
    }

    // Metric catalog (tsk213, P7.B6): seed the declared metrics, then reseed
    // when the extensions may have changed (config changes reseed through
    // the `config.metrics` reactor).
    {
        let metrics = state.metrics.clone();
        let changes = state.extension_catalog.changes();
        tokio::spawn(async move {
            metrics.run(changes).await;
        });
    }

    // Metric aggregate cube (tsk96), an asset (P7.B1): it reads the capture
    // and fact tables, so it folds what lands after each quiet burst of
    // commits to them; its first build here is the backfill. Purely an
    // accelerator — an unbuilt cube is a slow read, never a wrong one.
    state.assets.register(std::sync::Arc::new(
        crate::metric_cube::MetricCubeBuilder::new((*state.fact_store).clone())
            .with_visibility(state.metric_visibility.clone()),
    ));

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
            match state.metrics.rebuild_baseline(false).await {
                Ok(r) if r.ran => tracing::info!(
                    gauges = r.collectors_run,
                    failed = ?r.failed,
                    "metric tree baseline: complete",
                ),
                Ok(_) => tracing::debug!("metric tree baseline: nothing to do"),
                Err(e) => tracing::warn!(error = %e, "metric tree baseline failed"),
            }
        });
    }
}
