//! Golden tests: the EXACT response bodies the hook route returns to the
//! agent's harness. The wording steers the agent, so these pin it
//! byte-for-byte while enforcement and recording move behind the shared
//! `AgentPolicy` / `AgentContext` services (tsk281). `hook_marshaling.rs`
//! checks shapes; these check every byte.
//!
//! Goldens live in `tests/goldens/<name>.json` with the temp project path
//! written as `<ROOT>`. A mismatch fails; `UPDATE_GOLDENS=1` rewrites them
//! (then review the diff: a changed golden is a changed agent contract).

#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]
#![allow(clippy::unwrap_used)]

mod common;

use common::boot;

use oxplow_app::Services;
use oxplow_control_plane::ControlPlane;
use oxplow_db::EffortStore as _;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore as _;
use oxplow_tasks::{Task, TaskActorKind, TaskPriority, TaskStatus};

fn golden(name: &str, body: &serde_json::Value, root: &std::path::Path) {
    let text = serde_json::to_string_pretty(body)
        .unwrap()
        .replace(&root.to_string_lossy().to_string(), "<ROOT>");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(format!("{name}.json"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{text}\n")).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {name}; run with UPDATE_GOLDENS=1"));
    assert_eq!(text.trim_end(), want.trim_end(), "golden {name} changed");
}

async fn seed_thread(services: &Services, status: ThreadStatus) -> ThreadId {
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
        // The primary stream's worktree is the project itself.
        worktree_path: services.layout.project_dir.to_string_lossy().into(),
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
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    services.thread_store.upsert(&thread).await.unwrap();
    services
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
            thread.id, "claude",
        ))
        .await
        .unwrap();
    thread.id
}

/// An `in_progress` task on the thread, with its effort open.
async fn seed_task(services: &Services, thread_id: ThreadId, title: &str) -> TaskId {
    let now = Timestamp::from_unix_ms(1);
    let task = services
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread_id),
            parent_id: None,
            title: title.into(),
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
        })
        .await
        .unwrap();
    services
        .effort_store
        .start(&oxplow_tasks::work_item_ref(task), &thread_id, None)
        .await
        .unwrap();
    task
}

async fn post(
    cp: &ControlPlane,
    event: &str,
    thread: ThreadId,
    body: serde_json::Value,
) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .post(format!("{}/{}", cp.hook_base_url(), event))
        .header("authorization", format!("Bearer {}", cp.hook_token))
        .header("x-oxplow-thread", thread.to_string())
        .header("x-oxplow-stream", "str1")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

fn edit(path: &std::path::Path) -> serde_json::Value {
    serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": path.to_string_lossy() } })
}

#[tokio::test]
async fn write_guard_denies() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Queued).await;
    let with_path = post(&cp, "PreToolUse", tid, edit(&root.join("src/x.rs"))).await;
    golden("pre_tool_write_guard_path", &with_path, &root);
    let no_path = post(
        &cp,
        "PreToolUse",
        tid,
        serde_json::json!({ "tool_name": "Write", "tool_input": {} }),
    )
    .await;
    golden("pre_tool_write_guard_no_path", &no_path, &root);
}

/// The writer's edit is allowed with nothing tracked.
#[tokio::test]
async fn allowed_edit_and_post_tool_ack() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let allowed = post(&cp, "PreToolUse", tid, edit(&root.join("src/x.rs"))).await;
    golden("pre_tool_allowed", &allowed, &root);
    let mut after = edit(&root.join("src/x.rs"));
    after["tool_response"] = serde_json::json!({ "success": true });
    let ack = post(&cp, "PostToolUse", tid, after).await;
    golden("post_tool_edit_ack", &ack, &root);
}

/// A Stop is acked, whatever is open: oxplow never refuses one.
#[tokio::test]
async fn stop_ack() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_task(&svc, tid, "ship the thing").await;
    post(
        &cp,
        "UserPromptSubmit",
        tid,
        serde_json::json!({ "prompt": "do it", "session_id": "s1" }),
    )
    .await;
    post(&cp, "PreToolUse", tid, edit(&root.join("src/x.rs"))).await;
    let stop = post(
        &cp,
        "Stop",
        tid,
        serde_json::json!({ "session_id": "s1", "last_assistant_message": "Shipped." }),
    )
    .await;
    golden("stop_ack", &stop, &root);
}

#[tokio::test]
async fn prompt_context_first_then_deduped() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_task(&svc, tid, "ship the thing").await;
    let prompt = || serde_json::json!({ "prompt": "go", "session_id": "s1" });
    let first = post(&cp, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_first", &first, &root);
    let second = post(&cp, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_second", &second, &root);
}
