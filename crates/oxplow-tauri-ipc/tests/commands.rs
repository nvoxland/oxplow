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
    let v = commands::generated::app_version(app.state()).await.unwrap();
    assert!(!v.version.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn ping_returns_pong() {
    let app = TestApp::build();
    let v = commands::generated::ping(app.state()).await.unwrap();
    assert_eq!(v, "pong");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn log_ui_accepts_a_record() {
    let app = TestApp::build();
    commands::generated::log_ui(
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
    let streams = commands::generated::list_streams(app.state())
        .await
        .unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].kind, oxplow_domain::StreamKind::Primary);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_backlog_returns_empty_for_fresh_project() {
    let app = TestApp::build();
    let items = commands::generated::list_backlog(app.state())
        .await
        .unwrap();
    assert!(items.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_backlog_state_starts_at_zero() {
    let app = TestApp::build();
    let state = commands::generated::get_backlog_state(app.state())
        .await
        .unwrap();
    assert_eq!(state.items.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_threads_empty_for_unknown_stream() {
    let app = TestApp::build();
    let threads = commands::generated::list_threads(app.state(), StreamId::new(999999))
        .await
        .unwrap();
    assert!(threads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_closed_threads_empty_for_unknown_stream() {
    let app = TestApp::build();
    let threads = commands::generated::list_closed_threads(app.state(), StreamId::new(999999))
        .await
        .unwrap();
    assert!(threads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_task_missing_returns_none() {
    let app = TestApp::build();
    let item = commands::generated::get_task(app.state(), TaskId::new(999))
        .await
        .unwrap();
    assert!(item.is_none());
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
    let parent = commands::generated::create_task(
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
    let _child = commands::generated::create_task(
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
    let work_state = commands::generated::get_thread_work_state(app.state(), thread.id)
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
    let notes = commands::generated::list_wiki_pages(app.state())
        .await
        .unwrap();
    assert!(notes.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn search_wiki_titles_empty_input_returns_empty() {
    let app = TestApp::build();
    let hits = commands::generated::search_wiki_titles(app.state(), "".into(), 10)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recent_page_visits_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_recent_page_visits(app.state(), 10, None)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn top_visited_pages_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::top_visited_pages(app.state(), 10, None)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_code_quality_findings_empty_for_unknown_scan() {
    let app = TestApp::build();
    let v = commands::generated::list_code_quality_findings(app.state(), 9999)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_snapshots_empty_for_unknown_path() {
    let app = TestApp::build();
    let v = commands::generated::list_snapshots(app.state(), "nope.txt".into())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_snapshot_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::generated::get_snapshot(app.state(), 99999)
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_snapshot_summary_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::generated::get_snapshot_summary(app.state(), 99999)
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_hook_events_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_hook_events(app.state(), None, Some(10))
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_agent_statuses_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_agent_statuses(app.state())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_followups_empty_for_unknown_thread() {
    let app = TestApp::build();
    let v = commands::generated::list_followups(app.state(), ThreadId::new(999999))
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_background_tasks_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_background_tasks(app.state())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_config_returns_default_for_fresh_project() {
    let app = TestApp::build();
    let _ = commands::generated::get_config(app.state()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_workspace_entries_returns_root_listing() {
    let app = TestApp::build();
    let _entries = commands::generated::list_workspace_entries(app.state(), None, "".into())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn read_workspace_file_missing_path_errors() {
    let app = TestApp::build();
    let result = commands::generated::read_workspace_file(
        app.state(),
        None,
        "definitely-not-there.txt".into(),
    )
    .await;
    assert!(result.is_err());
}

// ---- Page-visit commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recently_finished_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_recently_finished(app.state(), None, 10)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clear_recently_finished_no_throw_on_empty() {
    let app = TestApp::build();
    commands::generated::clear_recently_finished(app.state(), None)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn count_page_visits_by_day_empty_for_fresh_project() {
    let app = TestApp::build();
    let days = commands::generated::count_page_visits_by_day(app.state(), 7)
        .await
        .unwrap();
    assert!(days.is_empty());
}

// ---- Wiki commands ----

// ---- Effort commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_task_efforts_empty_for_unknown_item() {
    let app = TestApp::build();
    let v = commands::generated::list_task_efforts(app.state(), TaskId::new(999))
        .await
        .unwrap();
    assert!(v.is_empty());
}

// ---------------------------------------------------------------------------
// Broad read-command coverage. The harness boots a real git repo + a primary
// stream + a default thread, so `stream_id: Option<String>` falls back to the
// primary worktree. Each test drives one more uncovered command adapter
// through the production `tauri::State` plumbing. Commands that can legitimately
// error on a fresh repo (no remote, missing path, unknown id) are called with
// `let _ =` so the test exercises the adapter without asserting a brittle
// outcome.
// ---------------------------------------------------------------------------

use oxplow_domain::stores::{StreamStore, ThreadStore};
use oxplow_domain::{EffortId, Stream, Thread};

/// Primary stream + its default thread, both of which `TestApp::build`
/// guarantees via `ensure_primary`.
async fn primary_and_thread(app: &TestApp) -> (Stream, Thread) {
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
    (stream, thread)
}

// ---- branch commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_branches_returns_default_branch() {
    let app = TestApp::build();
    let branches = commands::generated::list_branches(app.state())
        .await
        .unwrap();
    assert!(
        !branches.is_empty(),
        "a repo with one commit has at least its default branch"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_local_branches_returns_default_branch() {
    let app = TestApp::build();
    let branches = commands::generated::list_local_branches(app.state())
        .await
        .unwrap();
    assert!(!branches.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_default_branch_does_not_panic() {
    let app = TestApp::build();
    let _ = commands::generated::get_default_branch(app.state()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn delete_unknown_branch_errors() {
    let app = TestApp::build();
    let _ = commands::generated::delete_branch(app.state(), "no-such-branch".into(), false).await;
}

// ---- git read commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_reads_over_primary_worktree() {
    let app = TestApp::build();
    let s = app.state();
    let _ = commands::generated::get_repo_conflict_state(s.clone(), None).await;
    let _ =
        commands::generated::get_ahead_behind(s.clone(), None, "HEAD".into(), "HEAD".into()).await;
    let _ = commands::generated::list_all_refs(s.clone()).await;
    let _ = commands::generated::get_change_scopes(s.clone(), None).await;
    let _ = commands::generated::get_branch_changes(s.clone(), None, "HEAD".into()).await;
    let _ =
        commands::generated::read_file_at_ref(s.clone(), "HEAD".into(), "nope.txt".into()).await;
    let _ =
        commands::generated::list_file_commits(s.clone(), None, "nope.txt".into(), Some(10)).await;
    let _ = commands::generated::git_blame(s.clone(), None, "nope.txt".into()).await;
    let _ =
        commands::generated::local_blame(s.clone(), None, "nope.txt".into(), "a\nb\n".into()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_list_commands_return_empty_for_fresh_repo() {
    let app = TestApp::build();
    let s = app.state();
    assert!(
        commands::generated::list_recent_remote_branches(s.clone(), Some(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::search_workspace_text(s.clone(), None, "needle".into(), Some(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::resolve_commit_ref_labels(s.clone(), vec![])
            .await
            .unwrap()
            .is_empty()
    );
    let _ = commands::generated::list_adoptable_worktrees(s.clone())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_local_mutations_over_primary_worktree() {
    let app = TestApp::build();
    let s = app.state();
    // Append a gitignore entry, stage it, commit it — all local, no remote.
    let _ = commands::generated::append_to_gitignore(s.clone(), None, "target/".into()).await;
    let _ = commands::generated::git_add_path(s.clone(), None, ".gitignore".into()).await;
    let _ = commands::generated::git_commit_all(s.clone(), None, "add gitignore".into()).await;
    let _ = commands::generated::restore_path(s.clone(), None, ".gitignore".into()).await;
    let _ = commands::generated::git_merge_into(s.clone(), None, "HEAD".into()).await;
    let _ = commands::generated::git_rebase_onto(s.clone(), None, "HEAD".into()).await;
}

// ---- stream commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stream_reads_and_reorder() {
    let app = TestApp::build();
    let (stream, _) = primary_and_thread(&app).await;
    assert!(commands::generated::get_primary_stream(app.state())
        .await
        .unwrap()
        .is_some());
    let _ = commands::generated::get_current_stream(app.state())
        .await
        .unwrap();
    commands::generated::switch_stream(app.state(), Some(stream.id))
        .await
        .unwrap();
    commands::generated::reorder_streams(app.state(), vec![stream.id])
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn archive_unknown_stream_errors() {
    let app = TestApp::build();
    let _ = commands::generated::archive_stream(app.state(), StreamId::new(999999), false).await;
}

// ---- config commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn config_setters_round_trip() {
    use oxplow_config::AgentKind;
    let app = TestApp::build();
    commands::generated::set_agents(app.state(), vec![AgentKind::Claude, AgentKind::Codex])
        .await
        .unwrap();
    commands::generated::set_agent_prompt_append(app.state(), "be concise".into())
        .await
        .unwrap();
    commands::generated::set_snapshot_retention_days(app.state(), 30)
        .await
        .unwrap();
    commands::generated::set_snapshot_max_file_bytes(app.state(), 1_000_000)
        .await
        .unwrap();
    commands::generated::set_generated(
        app.state(),
        oxplow_config::GeneratedConfig {
            exclude: vec!["generated/".into()],
            include: vec![],
        },
    )
    .await
    .unwrap();
    let _ = commands::generated::get_workspace_context(app.state()).await;
}

// ---- thread commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn thread_reads_over_default_thread() {
    let app = TestApp::build();
    let (stream, _thread) = primary_and_thread(&app).await;
    assert!(!commands::generated::list_threads(app.state(), stream.id)
        .await
        .unwrap()
        .is_empty());
    let _ = commands::generated::get_thread_state(app.state(), stream.id)
        .await
        .unwrap();
}

// ---- comment commands (full round-trip) ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn comment_lifecycle_round_trip() {
    use commands::comments::CreateCommentRequest;
    use oxplow_domain::{CommentIntent, CommentStatus};
    let app = TestApp::build();
    let (stream, thread) = primary_and_thread(&app).await;

    assert!(
        commands::generated::list_comments_for_stream(app.state(), stream.id)
            .await
            .unwrap()
            .is_empty()
    );

    let c = commands::generated::create_comment(
        app.state(),
        CreateCommentRequest {
            stream_id: stream.id,
            thread_id: Some(thread.id),
            target_kind: "wiki".into(),
            target_id: "some-page".into(),
            quote: "the quote".into(),
            selectors_json: "{}".into(),
            context_chain: vec![],
            referenced_refs: vec![],
            intent: CommentIntent::Note,
            author: "tester".into(),
            body: "first message".into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        commands::generated::list_comments_for_stream(app.state(), stream.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        commands::generated::list_comments_for_target(
            app.state(),
            "wiki".into(),
            "some-page".into()
        )
        .await
        .unwrap()
        .len(),
        1
    );

    commands::generated::add_comment_message(
        app.state(),
        c.comment.id,
        "tester".into(),
        "reply".into(),
    )
    .await
    .unwrap();
    commands::generated::set_comment_intent(app.state(), c.comment.id, CommentIntent::Followup)
        .await
        .unwrap();
    commands::generated::set_comment_anchor(app.state(), c.comment.id, "{\"v\":1}".into(), true)
        .await
        .unwrap();
    commands::generated::relink_comment(
        app.state(),
        c.comment.id,
        "new quote".into(),
        "{\"v\":2}".into(),
    )
    .await
    .unwrap();
    commands::generated::set_comment_status(app.state(), c.comment.id, CommentStatus::Resolved)
        .await
        .unwrap();
    commands::generated::delete_comment(app.state(), c.comment.id)
        .await
        .unwrap();

    assert!(
        commands::generated::list_comments_for_stream(app.state(), stream.id)
            .await
            .unwrap()
            .is_empty()
    );
}

// ---- note commands (round-trip) ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn thread_note_round_trip() {
    let app = TestApp::build();
    let (_, thread) = primary_and_thread(&app).await;
    assert!(
        commands::generated::list_thread_notes(app.state(), thread.id)
            .await
            .unwrap()
            .is_empty()
    );
    let note = commands::generated::add_thread_note(
        app.state(),
        thread.id,
        "a finding".into(),
        "me".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        commands::generated::list_thread_notes(app.state(), thread.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let _ = commands::generated::list_task_events(app.state(), None, Some(thread.id))
        .await
        .unwrap();
    let _ = note;
}

// ---- page-ref + search + wiki-freshness reads ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn page_ref_reads_empty_for_fresh_project() {
    let app = TestApp::build();
    assert!(
        commands::generated::list_backlinks(app.state(), "wiki".into(), "slug".into(), None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::list_outbound(app.state(), "task".into(), "1".into(), Some(10))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn search_returns_empty_for_fresh_project() {
    let app = TestApp::build();
    assert!(
        commands::generated::search(app.state(), "anything".into(), None, None, Some(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(commands::generated::search(
        app.state(),
        "anything".into(),
        None,
        Some(vec!["wiki".into()]),
        None
    )
    .await
    .unwrap()
    .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wiki_freshness_reads_for_unknown_slug() {
    let app = TestApp::build();
    assert!(
        commands::generated::list_wiki_freshness(app.state(), "no-slug".into())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        commands::generated::mark_all_wiki_refs_verified(app.state(), "no-slug".into())
            .await
            .unwrap(),
        0
    );
    let _ = commands::generated::mark_wiki_ref_verified(app.state(), "no-slug".into(), "p".into())
        .await;
}

// ---- effort reads ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn effort_reads_empty_for_unknown_ids() {
    let app = TestApp::build();
    assert!(
        commands::generated::get_effort_files(app.state(), EffortId::new(999))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::list_efforts_at_snapshots(app.state(), vec![])
            .await
            .unwrap()
            .is_empty()
    );
    {
        let split =
            commands::generated::list_changed_paths_for_effort(app.state(), EffortId::new(999))
                .await
                .unwrap();
        assert!(split.claimed.is_empty() && split.unclaimed.is_empty());
    }
    assert!(
        commands::generated::list_effort_observations(app.state(), EffortId::new(999), None)
            .await
            .unwrap()
            .is_empty()
    );
}

// ---- snapshot reads ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn snapshot_reads_empty_for_fresh_project() {
    let app = TestApp::build();
    let (stream, _) = primary_and_thread(&app).await;
    assert!(
        commands::generated::list_file_snapshots_for_stream(app.state(), stream.id, Some(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::list_snapshots_for_stream(app.state(), stream.id, Some(10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::list_snapshot_change_entries(app.state(), 999)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::read_snapshot_file_content(app.state(), 999)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        commands::generated::list_files_for_snapshot(app.state(), 999)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        commands::generated::list_wiki_slugs_for_snapshots(app.state(), vec![999])
            .await
            .unwrap()
            .is_empty()
    );
    let _ = commands::generated::get_blob_storage_bytes(app.state())
        .await
        .unwrap();
    let _ = commands::generated::get_snapshot_stats(app.state(), 999).await;
    let _ = commands::generated::get_snapshot_pair_diff(app.state(), None, None).await;
    let _ = commands::generated::restore_file_from_snapshot(app.state(), 999).await;
}

// ---- workspace reads + file round-trip ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn workspace_reads_and_file_round_trip() {
    let app = TestApp::build();
    let _ = commands::generated::list_workspace_files(app.state(), None)
        .await
        .unwrap();
    let _ = commands::generated::get_workspace_status_summary(app.state(), None)
        .await
        .unwrap();
    let _ = commands::generated::read_file(
        app.state(),
        None,
        "made-up.txt".into(),
        oxplow_tree_source::TreeVersion::Disk,
    )
    .await;
    // Create → read → rename → delete a file inside the worktree.
    commands::generated::write_workspace_file(
        app.state(),
        None,
        "scratch.txt".into(),
        "hello".into(),
    )
    .await
    .unwrap();
    let f = commands::generated::read_workspace_file(app.state(), None, "scratch.txt".into())
        .await
        .unwrap();
    assert!(f.content.contains("hello"));
    let _ =
        commands::generated::create_workspace_directory(app.state(), None, "subdir".into()).await;
    let _ = commands::generated::rename_workspace_path(
        app.state(),
        None,
        "scratch.txt".into(),
        "scratch2.txt".into(),
    )
    .await;
    let _ =
        commands::generated::delete_workspace_path(app.state(), None, "scratch2.txt".into()).await;
}

// ---- lsp list reads ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn lsp_list_reads_for_fresh_project() {
    let app = TestApp::build();
    assert!(
        commands::generated::list_installed_lsp_packages(app.state())
            .await
            .unwrap()
            .is_empty()
    );
    let _ = commands::generated::list_lsp_servers(app.state()).await;
}

// ---------------------------------------------------------------------------
// analyze_functions — the one richly-assertable code-quality core. The Tauri
// adapter now takes `Services` only to read the project's zone table (tsk251),
// so these drive the core directly with an empty table: they lock the real
// parse + churn-attribution behavior the Change Analysis dashboard depends on,
// not just "doesn't panic". Zone classification has its own tests in
// oxplow-rpc / oxplow-code-deps.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn analyze_functions_detects_functions_and_churn_for_a_modified_file() {
    use commands::code_quality::AnalyzeFileSpec;
    // head changes alpha's body AND adds beta.
    let base = "fn alpha() -> i32 {\n    1\n}\n";
    let head = "fn alpha() -> i32 {\n    2\n}\n\nfn beta() -> i32 {\n    3\n}\n";
    let result = oxplow_rpc::commands::code_quality::analyze_functions(
        vec![AnalyzeFileSpec {
            path: "src/x.rs".into(),
            base_content: Some(base.into()),
            head_content: Some(head.into()),
        }],
        &[],
    )
    .await
    .unwrap();

    let base_side = result
        .sides
        .iter()
        .find(|s| s.side == "base")
        .expect("base side present");
    let head_side = result
        .sides
        .iter()
        .find(|s| s.side == "head")
        .expect("head side present");
    assert!(base_side.functions.iter().any(|f| f.name == "alpha"));
    assert!(head_side.functions.iter().any(|f| f.name == "alpha"));
    assert!(
        head_side.functions.iter().any(|f| f.name == "beta"),
        "the newly added fn must appear on the head side"
    );

    assert_eq!(result.churn.len(), 1, "one modified file → one churn entry");
    let churn = &result.churn[0];
    assert_eq!(churn.path, "src/x.rs");
    assert!(
        churn.file_added > 0,
        "adding fn beta should register added lines, got {churn:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn analyze_functions_added_file_has_only_head_side_and_no_churn() {
    use commands::code_quality::AnalyzeFileSpec;
    let result = oxplow_rpc::commands::code_quality::analyze_functions(
        vec![AnalyzeFileSpec {
            path: "src/new.rs".into(),
            base_content: None,
            head_content: Some("fn brand_new() {}\n".into()),
        }],
        &[],
    )
    .await
    .unwrap();

    assert_eq!(
        result.sides.len(),
        1,
        "an added file has only the head side"
    );
    assert_eq!(result.sides[0].side, "head");
    assert!(result.sides[0]
        .functions
        .iter()
        .any(|f| f.name == "brand_new"));
    assert!(
        result.churn.is_empty(),
        "a file with no base content has no before→after churn"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn analyze_functions_empty_input_returns_empty() {
    let result = oxplow_rpc::commands::code_quality::analyze_functions(vec![], &[])
        .await
        .unwrap();
    assert!(result.sides.is_empty());
    assert!(result.churn.is_empty());
    assert!(result.import_deltas.is_empty());
}

// ---- launcher: recent-projects exists-flag mapping ----

/// `list_recent_projects` layers an `exists` flag onto each stored row.
/// This is the only crate-local logic in `launch.rs` (the rest spawns
/// processes), and the launcher's "missing" badge depends on it. Build a
/// mock app managing a `RecentProjectsState` and assert the flag tracks
/// whether the directory is still on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_recent_projects_flags_missing_directories() {
    use oxplow_config::RecentProjects;
    use oxplow_tauri_ipc::RecentProjectsState;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;

    let state_dir = tempfile::TempDir::new().unwrap();
    let recent: RecentProjectsState =
        Arc::new(RecentProjects::new(state_dir.path().join("recent.json")));

    // A live project dir, plus one we delete after recording so its row
    // points at a now-missing directory.
    let live = tempfile::TempDir::new().unwrap();
    let live_canon = std::fs::canonicalize(live.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    recent.record(live.path());

    let gone = tempfile::TempDir::new().unwrap();
    recent.record(gone.path());
    drop(gone); // directory removed; the recorded row remains

    let app = mock_builder()
        .manage(recent.clone())
        .build(mock_context(noop_assets()))
        .unwrap();

    let views = commands::launch::list_recent_projects(app.state::<RecentProjectsState>())
        .await
        .unwrap();

    assert_eq!(views.len(), 2);
    let live_view = views
        .iter()
        .find(|v| v.path == live_canon)
        .expect("live project row present");
    assert!(
        live_view.exists,
        "an existing directory must be flagged exists=true"
    );
    let gone_view = views
        .iter()
        .find(|v| v.path != live_canon)
        .expect("missing project row present");
    assert!(
        !gone_view.exists,
        "a deleted directory must be flagged exists=false"
    );
}
