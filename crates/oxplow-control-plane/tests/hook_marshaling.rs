//! HTTP-level tests for the hook surface: auth, envelope routing, and
//! the exact JSON shapes marshaled back to Claude Code (PreToolUse
//! deny bodies, Stop directives). These pin the wire contract the
//! Claude Code plugin depends on — the unit tests in lib.rs cover the
//! helpers, but nothing else exercises `handle_hook` end to end.

// Test-only crate: terse unwraps are the assertion style here (the
// clippy.toml allow-unwrap-in-tests carve-out doesn't reach helper
// fns in integration-test crates).
#![allow(clippy::unwrap_used)]

use std::process::Command;
use std::sync::Arc;

use oxplow_app::Services;
use oxplow_control_plane::{spawn, ControlPlane};
use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
use oxplow_domain::{
    Stream, StreamId, StreamKind, Task, TaskActorKind, TaskId, TaskPriority, TaskStatus, Thread,
    ThreadId, ThreadStatus, Timestamp,
};

async fn boot() -> (
    ControlPlane,
    Arc<Services>,
    std::path::PathBuf,
    tempfile::TempDir,
) {
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
    // Canonicalize: macOS tempdirs live behind the /var → /private/var
    // symlink, and the write-guard's is-inside-project check compares
    // literal path prefixes.
    let root = dir.path().canonicalize().unwrap();
    let services = Arc::new(Services::in_memory(&root).unwrap());
    let cp = spawn(services.clone()).await.unwrap();
    (cp, services, root, dir)
}

/// Seed a stream + thread with the given status; returns the thread id
/// string used in the X-Oxplow-Thread header.
async fn seed_thread(services: &Services, status: ThreadStatus) -> ThreadId {
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
        worktree_path: "/p".into(),
        working_pane: String::new(),
        talking_pane: String::new(),
        working_session_id: String::new(),
        talking_session_id: String::new(),
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    services.stream_store.upsert(&stream).await.unwrap();
    let thread = Thread {
        id: ThreadId::new(1),
        stream_id: stream.id,
        title: "t".into(),
        status,
        sort_index: 0,
        pane_target: "working".into(),
        agent: oxplow_domain::AgentKind::Claude,
        resume_session_id: String::new(),
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    services.thread_store.upsert(&thread).await.unwrap();
    thread.id
}

async fn seed_in_progress_task(services: &Services, thread_id: ThreadId) {
    let now = Timestamp::from_unix_ms(1);
    let task = Task {
        id: TaskId::placeholder(),
        thread_id: Some(thread_id),
        parent_id: None,
        title: "ship the thing".into(),
        description: "d".into(),
        status: TaskStatus::InProgress,
        priority: TaskPriority::Medium,
        sort_index: 0,
        created_by: TaskActorKind::User,
        created_at: now,
        updated_at: now,
        completed_at: None,
        deleted_at: None,
        note_count: 0,
        author: None,
    };
    services.task_store.insert(&task).await.unwrap();
}

fn hook_url(cp: &ControlPlane, event: &str) -> String {
    format!("{}/{}", cp.hook_base_url(), event)
}

async fn post_hook(
    cp: &ControlPlane,
    event: &str,
    thread: Option<ThreadId>,
    body: serde_json::Value,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(hook_url(cp, event))
        .header("authorization", format!("Bearer {}", cp.hook_token))
        .json(&body);
    if let Some(t) = thread {
        req = req.header("x-oxplow-thread", t.to_string());
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn hook_post_without_bearer_is_unauthorized() {
    let (cp, _svc, _root, _dir) = boot().await;
    let resp = reqwest::Client::new()
        .post(hook_url(&cp, "Stop"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn unknown_hook_event_is_acked_not_persisted() {
    let (cp, _svc, _root, _dir) = boot().await;
    let resp = post_hook(&cp, "TotallyNovelEvent", None, serde_json::json!({})).await;
    // Claude Code's HTTP hooks treat anything but 200 as a failure and
    // print a "non-blocking status code" warning into the agent's
    // terminal — every ack path must be a plain 200.
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn session_start_resets_and_acks() {
    let (cp, _svc, _root, _dir) = boot().await;
    let resp = post_hook(
        &cp,
        "SessionStart",
        None,
        serde_json::json!({ "session_id": "s1" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn pre_tool_use_on_read_only_thread_denies_with_write_guard_shape() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Queued).await;
    let target = root.join("src/x.rs");
    let resp = post_hook(
        &cp,
        "PreToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let out = &body["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse");
    assert_eq!(out["permissionDecision"], "deny");
    let reason = out["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("read-only"), "unexpected reason: {reason}");
}

#[tokio::test]
async fn pre_tool_use_without_in_progress_task_denies_with_filing_shape() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let target = root.join("src/x.rs");
    let resp = post_hook(
        &cp,
        "PreToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let out = &body["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse");
    assert_eq!(out["permissionDecision"], "deny");
    let reason = out["permissionDecisionReason"].as_str().unwrap();
    assert!(
        reason.contains("requires a tracked task"),
        "unexpected reason: {reason}"
    );
}

#[tokio::test]
async fn pre_tool_use_with_in_progress_task_is_allowed() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_in_progress_task(&svc, tid).await;
    let target = root.join("src/x.rs");
    let resp = post_hook(
        &cp,
        "PreToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
        }),
    )
    .await;
    // Allowed calls fall through to the generic ack.
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({}));
}

#[tokio::test]
async fn stop_with_in_progress_task_returns_block_directive() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_in_progress_task(&svc, tid).await;

    // Open a turn, then register write activity inside it so the Stop
    // pipeline's Q&A-turn carve-out doesn't suppress the audit.
    post_hook(
        &cp,
        "UserPromptSubmit",
        Some(tid),
        serde_json::json!({ "prompt": "do the thing", "session_id": "s1" }),
    )
    .await;
    let target = root.join("src/x.rs");
    post_hook(
        &cp,
        "PreToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
        }),
    )
    .await;

    let resp = post_hook(&cp, "Stop", Some(tid), serde_json::json!({})).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["decision"], "block");
    let reason = body["reason"].as_str().unwrap();
    assert!(reason.contains("AUDIT"), "unexpected reason: {reason}");
    assert!(
        reason.contains("ship the thing"),
        "directive should name the open task: {reason}"
    );

    // A second Stop with an unchanged in_progress set must NOT repeat
    // the audit (signature dedup) — it falls through to the plain ack.
    post_hook(
        &cp,
        "UserPromptSubmit",
        Some(tid),
        serde_json::json!({ "prompt": "again", "session_id": "s1" }),
    )
    .await;
    post_hook(
        &cp,
        "PreToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
        }),
    )
    .await;
    let resp2 = post_hook(&cp, "Stop", Some(tid), serde_json::json!({})).await;
    assert_eq!(resp2.status(), 200);
}

#[tokio::test]
async fn post_tool_use_edit_acks_200_empty() {
    // The observed regression: every Edit's PostToolUse fell through
    // to a 202 ack, and Claude Code printed "PostToolUse:Edit hook
    // error ... non-blocking status code" into the agent terminal on
    // every single edit.
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_in_progress_task(&svc, tid).await;
    let target = root.join("src/x.rs");
    let resp = post_hook(
        &cp,
        "PostToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": target.to_string_lossy() },
            "session_id": "s1",
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({}));
}

#[tokio::test]
async fn ingest_failure_still_acks_200() {
    // An unknown thread id makes agent_turn's thread FK fail inside
    // ingest. The agent can't do anything useful with a 500 — it just
    // prints the warning line — so the handler logs server-side and
    // acks 200 {} anyway.
    let (cp, _svc, _root, _dir) = boot().await;
    let bogus = ThreadId::new(999_999);
    let resp = post_hook(
        &cp,
        "UserPromptSubmit",
        Some(bogus),
        serde_json::json!({ "prompt": "hello", "session_id": "s1" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({}));
}
