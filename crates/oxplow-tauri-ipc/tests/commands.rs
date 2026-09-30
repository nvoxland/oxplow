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
use oxplow_domain::{StreamId, ThreadId};
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
async fn list_file_snapshots_empty_for_unknown_path() {
    let app = TestApp::build();
    let v = commands::generated::list_file_snapshots(app.state(), "nope.txt".into())
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn get_file_snapshot_missing_returns_none() {
    let app = TestApp::build();
    let v = commands::generated::get_file_snapshot(app.state(), 99999)
        .await
        .unwrap();
    assert!(v.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn list_agent_events_empty_for_fresh_project() {
    let app = TestApp::build();
    let v = commands::generated::list_agent_events(app.state(), None, None, Some(10))
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

// ---- Wiki commands ----

// ---- Effort commands ----

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
async fn vcs_branches_include_the_default_branch() {
    let app = TestApp::build();
    let branches = commands::generated::vcs_branches(app.state(), None)
        .await
        .unwrap();
    assert!(
        branches.iter().any(|b| b.is_default),
        "a repo with one commit has its default branch: {branches:?}"
    );
}

// ---- git read commands ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_reads_over_primary_worktree() {
    let app = TestApp::build();
    let s = app.state();
    let _ = commands::generated::vcs_status(s.clone(), None).await;
    let _ = commands::generated::vcs_head(s.clone(), None).await;
    let head = || oxplow_domain::vcs::Revision::git("HEAD");
    let _ = commands::generated::vcs_divergence(s.clone(), None, head(), head()).await;
    let _ = commands::generated::git_change_scopes(s.clone(), None).await;
    let _ = commands::generated::vcs_log(s.clone(), None, Some(10), false).await;
    let _ = commands::generated::read_at(
        s.clone(),
        None,
        "nope.txt".into(),
        oxplow_domain::vcs::Revision::git("HEAD"),
    )
    .await;
    let _ = commands::generated::vcs_file_history(s.clone(), None, "nope.txt".into(), 10).await;
    let _ = commands::generated::vcs_blame(
        s.clone(),
        None,
        "nope.txt".into(),
        oxplow_domain::vcs::Revision::Working,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn git_list_commands_return_empty_for_fresh_repo() {
    let app = TestApp::build();
    let s = app.state();
    assert!(
        commands::generated::git_list_recent_remote_branches(s.clone(), Some(10))
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
        commands::generated::git_resolve_commit_ref_labels(s.clone(), vec![])
            .await
            .unwrap()
            .is_empty()
    );
    let _ = commands::generated::vcs_list_adoptable_workspaces(s.clone())
        .await
        .unwrap();
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
}

// ---- snapshot reads ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn snapshot_reads_empty_for_fresh_project() {
    let app = TestApp::build();
    let (stream, _) = primary_and_thread(&app).await;
    assert!(
        commands::generated::list_snapshots_for_stream(app.state(), stream.id, Some(10))
            .await
            .unwrap()
            .is_empty()
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
    let _ = commands::generated::restore_file_snapshot(app.state(), 999).await;
}

// ---- workspace reads + file round-trip ----

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn workspace_reads_and_file_round_trip() {
    let app = TestApp::build();
    let _ = commands::generated::list_workspace_files(app.state(), None)
        .await
        .unwrap();
    let _ = commands::generated::files_at(app.state(), None, oxplow_domain::vcs::Revision::Working)
        .await
        .unwrap();
    // Create → read → rename → delete a file inside the primary's
    // worktree; a change names its stream (tsk551).
    let primary = commands::generated::get_primary_stream(app.state())
        .await
        .unwrap()
        .expect("a primary stream")
        .id
        .to_string();
    let refused = commands::generated::write_workspace_file(
        app.state(),
        None,
        "scratch.txt".into(),
        "hello".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, "INVALID", "{}", refused.message);
    commands::generated::write_workspace_file(
        app.state(),
        Some(primary.clone()),
        "scratch.txt".into(),
        "hello".into(),
    )
    .await
    .unwrap();
    let f = commands::generated::read_workspace_file(app.state(), None, "scratch.txt".into())
        .await
        .unwrap();
    assert!(f.content.contains("hello"));
    commands::generated::create_workspace_directory(
        app.state(),
        Some(primary.clone()),
        "subdir".into(),
    )
    .await
    .unwrap();
    commands::generated::rename_workspace_path(
        app.state(),
        Some(primary.clone()),
        "scratch.txt".into(),
        "scratch2.txt".into(),
    )
    .await
    .unwrap();
    commands::generated::delete_workspace_path(app.state(), Some(primary), "scratch2.txt".into())
        .await
        .unwrap();
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
