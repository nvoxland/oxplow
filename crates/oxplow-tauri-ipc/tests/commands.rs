// Integration-test code — `unwrap()` is idiomatic here; relax the
// workspace `unwrap_used` guardrail (clippy.toml only exempts unit-test
// modules, not `tests/` helper fns).
#![allow(clippy::unwrap_used)]

//! Integration coverage for the `#[tauri::command]` adapters.
//!
//! Each test builds a fresh `TestApp` (Services with in-memory DB
//! plus a Tauri mock runtime) and invokes commands through
//! `tauri::State`. Goal: bring the per-crate floor for
//! `oxplow-tauri-ipc/src/commands/*` off 0% and lock the
//! argument-shape + error-mapping seam against silent regressions
//! (`state.unwrap()` panics, type mismatches between renderer and
//! Rust signatures, etc.).

mod harness;

use harness::TestApp;
use oxplow_domain::{StreamId, TaskId, ThreadId};
use oxplow_tauri_ipc::commands;

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn app_version_returns_pkg_version() {
    let app = TestApp::build();
    let v = commands::app::app_version(app.state()).await.unwrap();
    assert!(!v.version.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn ping_returns_pong() {
    let app = TestApp::build();
    let v = commands::app::ping(app.state()).await.unwrap();
    assert_eq!(v, "pong");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn log_ui_accepts_a_record() {
    let app = TestApp::build();
    commands::app::log_ui(
        app.state(),
        commands::app::UiLogEntry {
            level: "info".into(),
            message: "hello from test".into(),
            context: Some("{\"k\":\"v\"}".into()),
            client_id: None,
            timestamp: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_streams_returns_primary_for_fresh_project() {
    // TestApp boots Services::in_memory, which now calls
    // ensure_primary so the snapshot capture singleton has a stream
    // to bind to. A fresh project therefore has exactly one stream.
    let app = TestApp::build();
    let streams = commands::streams::list_streams(app.state()).await.unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].kind, oxplow_domain::StreamKind::Primary);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_backlog_returns_empty_for_fresh_project() {
    let app = TestApp::build();
    let items = commands::backlog::list_backlog(app.state()).await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_backlog_state_starts_at_zero() {
    let app = TestApp::build();
    let state = commands::backlog::get_backlog_state(app.state())
        .await
        .unwrap();
    assert_eq!(state.items.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_threads_empty_for_unknown_stream() {
    let app = TestApp::build();
    let threads = commands::threads::list_threads(app.state(), StreamId::new(999999))
        .await
        .unwrap();
    assert!(threads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_closed_threads_empty_for_unknown_stream() {
    let app = TestApp::build();
    let threads = commands::threads::list_closed_threads(app.state(), StreamId::new(999999))
        .await
        .unwrap();
    assert!(threads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_task_missing_returns_none() {
    let app = TestApp::build();
    let item = commands::tasks::get_task(app.state(), TaskId::new(999))
        .await
        .unwrap();
    assert!(item.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_tasks_for_thread_empty() {
    let app = TestApp::build();
    let items = commands::tasks::list_tasks_for_thread(app.state(), ThreadId::new(999999))
        .await
        .unwrap();
    assert!(items.is_empty());
}

/// End-to-end: a task with at least one child lands in
/// `ThreadWorkState.epics`, NOT in `items`. The frontend's
/// `computeActiveEpicContext` relies on this bucketing — if a parent
/// row drops into `items` instead, the rail's "Active epic" affordance
/// silently goes away. Drive the IPC create_task path twice (parent
/// + child) and read back the bucketed work state.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_thread_work_state_buckets_parents_with_children_as_epics() {
    use oxplow_app::CreateTaskInput;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    let app = TestApp::build();
    // The boot wires up a primary stream + its default active thread
    // via ensure_primary. Reuse them rather than insert a second
    // primary (the unique partial index would reject it).
    let stream = app.state.stream_store.primary().await.unwrap().unwrap();
    let thread = app
        .state
        .thread_store
        .list_for_stream(&stream.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("primary stream should have a default thread");
    let parent = commands::tasks::create_task(
        app.state(),
        commands::tasks::CreateTaskRequest {
            thread_id: Some(thread.id),
            input: CreateTaskInput {
                title: "parent".into(),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    let _child = commands::tasks::create_task(
        app.state(),
        commands::tasks::CreateTaskRequest {
            thread_id: Some(thread.id),
            input: CreateTaskInput {
                title: "child".into(),
                parent_id: Some(parent.id),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    let work_state = commands::threads::get_thread_work_state(app.state(), thread.id)
        .await
        .unwrap();
    assert!(
        work_state.epics.iter().any(|e| e.id == parent.id),
        "parent should appear in epics: {:?}",
        work_state.epics
    );
    assert!(
        !work_state.items.iter().any(|i| i.id == parent.id),
        "parent should NOT appear in items: {:?}",
        work_state.items
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_wiki_pages_empty_for_fresh_project() {
    let app = TestApp::build();
    let notes = commands::wiki::list_wiki_pages(app.state()).await.unwrap();
    assert!(notes.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn search_wiki_titles_empty_input_returns_empty() {
    let app = TestApp::build();
    let hits = commands::wiki::search_wiki_titles(app.state(), "".into(), 10)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recent_page_visits_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::page_visit::list_recent_page_visits(app.state(), 10, None)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn top_visited_pages_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::page_visit::top_visited_pages(app.state(), 10, None)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recent_usage_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::usage::list_recent_usage(app.state(), 10)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_code_quality_scans_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::code_quality::list_code_quality_scans(app.state(), 10)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_code_quality_findings_empty_for_unknown_scan() {
    let app = TestApp::build();
    let v = commands::code_quality::list_code_quality_findings(app.state(), 9999)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_snapshots_empty_for_unknown_path() {
    let app = TestApp::build();
    let v = commands::snapshot::list_snapshots(app.state(), "nope.txt".into())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_snapshot_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::snapshot::get_snapshot(app.state(), 99999)
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_snapshot_summary_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::snapshot::get_snapshot_summary(app.state(), 99999)
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_hook_events_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::hooks::list_hook_events(app.state(), None, Some(10))
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_agent_statuses_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::hooks::list_agent_statuses(app.state())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_followups_empty_for_unknown_thread() {
    let app = TestApp::build();
    let v = commands::followup::list_followups(app.state(), ThreadId::new(999999))
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_background_tasks_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::background::list_background_tasks(app.state())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_config_returns_default_for_fresh_project() {
    let app = TestApp::build();
    let _ = commands::config::get_config(app.state()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_workspace_entries_returns_root_listing() {
    let app = TestApp::build();
    let _entries = commands::workspace::list_workspace_entries(app.state(), None, "".into())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn read_workspace_file_missing_path_errors() {
    let app = TestApp::build();
    let result = commands::workspace::read_workspace_file(
        app.state(),
        None,
        "definitely-not-there.txt".into(),
    )
    .await;
    assert!(result.is_err());
}

// ---- Page-visit commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_currently_open_usage_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::page_visit::list_currently_open_usage(app.state(), 10)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recently_finished_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::page_visit::list_recently_finished(app.state(), None, 10)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clear_recently_finished_no_throw_on_empty() {
    let app = TestApp::build();
    commands::page_visit::clear_recently_finished(app.state(), None)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn count_page_visits_by_day_empty_for_fresh_project() {
    let app = TestApp::build();
    let days = commands::page_visit::count_page_visits_by_day(app.state(), 7)
        .await
        .unwrap();
    assert!(days.is_empty());
}

// ---- Wiki commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_wiki_page_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::wiki::get_wiki_page(app.state(), "no-such-slug".into())
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn search_wiki_bodies_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::wiki::search_wiki_bodies(app.state(), "any".into(), 20)
        .await
        .unwrap();
    assert!(v.is_empty());
}

// ---- task commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_tasks_for_thread_returns_empty_again() {
    // Slightly different from the existing list_tasks_for_thread_empty
    // helper — exercises the same surface with an explicit ThreadId conversion.
    let app = TestApp::build();
    let v = commands::tasks::list_tasks_for_thread(app.state(), ThreadId::new(999998))
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_task_summaries_for_empty_thread() {
    let app = TestApp::build();
    let v = commands::tasks::get_task_summaries(app.state(), Some(ThreadId::new(999998)))
        .await
        .unwrap();
    assert!(v.is_empty());
}

// ---- Effort commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_task_efforts_empty_for_unknown_item() {
    let app = TestApp::build();
    let v = commands::effort::list_task_efforts(app.state(), TaskId::new(999))
        .await
        .unwrap();
    assert!(v.is_empty());
}
