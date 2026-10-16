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

use common::{boot, seed_worktree_stream};

use oxplow_app::Services;
use oxplow_control_plane::ControlPlane;
use oxplow_db::EffortStore as _;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, Timestamp};
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

async fn seed_thread(services: &Services) -> ThreadId {
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
        host: oxplow_domain::HostId::LOCAL,
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
        sort_index: 0,
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
        mark: None,
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
    svc: &Services,
    event: &str,
    thread: ThreadId,
    body: serde_json::Value,
) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .post(format!("{}/{}", cp.hook_base_url(), event))
        .bearer_auth(common::bearer(svc, thread).await)
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

/// No thread edits another stream's worktree: the deny names it.
#[tokio::test]
async fn the_guard_denies_another_streams_worktree() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    let other = seed_worktree_stream(&svc, &root).await;
    let denied = post(&cp, &svc, "PreToolUse", tid, edit(&other.join("src/x.rs"))).await;
    golden("pre_tool_foreign_worktree", &denied, &root);
}

/// An edit in its own worktree is allowed with nothing tracked.
#[tokio::test]
async fn allowed_edit_and_post_tool_ack() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    let allowed = post(&cp, &svc, "PreToolUse", tid, edit(&root.join("src/x.rs"))).await;
    golden("pre_tool_allowed", &allowed, &root);
    let mut after = edit(&root.join("src/x.rs"));
    after["tool_response"] = serde_json::json!({ "success": true });
    let ack = post(&cp, &svc, "PostToolUse", tid, after).await;
    golden("post_tool_edit_ack", &ack, &root);
}

/// A Stop is acked, whatever is open: oxplow never refuses one.
#[tokio::test]
async fn stop_ack() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    seed_task(&svc, tid, "ship the thing").await;
    post(
        &cp,
        &svc,
        "UserPromptSubmit",
        tid,
        serde_json::json!({ "prompt": "do it", "session_id": "s1" }),
    )
    .await;
    post(&cp, &svc, "PreToolUse", tid, edit(&root.join("src/x.rs"))).await;
    let stop = post(
        &cp,
        &svc,
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
    let tid = seed_thread(&svc).await;
    seed_task(&svc, tid, "ship the thing").await;
    let prompt = || serde_json::json!({ "prompt": "go", "session_id": "s1" });
    let first = post(&cp, &svc, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_first", &first, &root);
    let second = post(&cp, &svc, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_second", &second, &root);
}

/// The session context names the other live agents in the worktree: one
/// on another thread of the stream, its process running.
#[tokio::test]
async fn prompt_context_names_the_other_agents_in_the_worktree() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    let now = Timestamp::from_unix_ms(1);
    let other = Thread {
        id: ThreadId::new(2),
        stream_id: StreamId::new(1),
        title: "other".into(),
        sort_index: 1,
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
        mark: None,
    };
    svc.thread_store.upsert(&other).await.unwrap();
    let mut new = oxplow_domain::agent_session::NewAgentSession::terminal(other.id, "codex");
    new.title = "Login fix".into();
    let session = svc.agent_session_store.open(&new).await.unwrap().id;
    // This test process stands in for the other agent's.
    let me = std::process::id();
    svc.db
        .transaction(move |tx| {
            oxplow_db::agent_session_store::set_pid_tx(tx, session, me, Timestamp::now())
        })
        .await
        .unwrap();
    let out = post(
        &cp,
        &svc,
        "UserPromptSubmit",
        tid,
        serde_json::json!({ "prompt": "go", "session_id": "s1" }),
    )
    .await;
    golden("prompt_context_with_others", &out, &root);
}

/// Every harness core runs renders the write guard's deny as the golden:
/// the answer is core's, the shape the harness's (Codex's hooks and
/// opencode's bridge speak Claude's), so a session of any of them gets
/// these bytes.
#[tokio::test]
async fn every_built_in_harness_renders_the_golden_deny() {
    let (_cp, svc, _root, _dir) = boot().await;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/pre_tool_foreign_worktree.json");
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let reason = golden["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
        .to_string();
    let harnesses = svc.harnesses.all();
    assert!(
        harnesses.len() >= 3,
        "foundation's harnesses are registered"
    );
    for h in harnesses {
        assert_eq!(
            h.render(&oxplow_domain::agent::observe::HookAnswer::Deny {
                reason: reason.clone()
            })
            .await,
            golden,
            "{}",
            h.id()
        );
    }
}

/// Post `event` as agent session `bearer` (not the thread's newest).
async fn post_as(
    cp: &ControlPlane,
    bearer: &str,
    event: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .post(format!("{}/{}", cp.hook_base_url(), event))
        .bearer_auth(bearer)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

/// A harness whose hook answers never reach its agent (one that declares
/// neither `prompt_context` nor `tool_context`: here a harness no
/// capability names) is answered with an ack, and nothing meant for its agent
/// is taken: a nudge stays undelivered rather than being stamped and lost.
#[tokio::test]
async fn a_harness_without_hook_context_is_acked_and_keeps_its_nudges() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    let session = svc
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
            tid, "bare",
        ))
        .await
        .unwrap();
    let bearer = common::bearer_for(&svc, session.id).await;
    svc.nudge_store
        .record(oxplow_db::agent_nudge_store::NewAgentNudge {
            thread_id: tid.to_string(),
            kind: "report-less-run".into(),
            message: "Your test run wrote no report oxplow could read.".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let prompt = post_as(
        &cp,
        &bearer,
        "UserPromptSubmit",
        serde_json::json!({ "prompt": "go", "session_id": "o1" }),
    )
    .await;
    assert_eq!(prompt, serde_json::json!({}));
    let mut after = edit(&root.join("src/x.rs"));
    after["session_id"] = "o1".into();
    after["tool_response"] = serde_json::json!({ "success": true });
    assert_eq!(
        post_as(&cp, &bearer, "PostToolUse", after).await,
        serde_json::json!({})
    );
    let waiting = svc
        .nudge_store
        .take_for_agent(&tid.to_string(), 2_000)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1, "the nudge is still undelivered");
}

/// After a compaction the agent's context is fresh: the next prompt
/// carries the session context (and decisions) again, though they haven't
/// changed. Claude Code delivers `PreCompact` (its HTTP `SessionStart`
/// never arrives).
#[tokio::test]
async fn a_compaction_brings_the_context_back_on_the_next_prompt() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc).await;
    let prompt = || serde_json::json!({ "prompt": "go", "session_id": "s1" });
    let first = post(&cp, &svc, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_first", &first, &root);
    assert_eq!(
        post(&cp, &svc, "UserPromptSubmit", tid, prompt()).await,
        serde_json::json!({})
    );
    let compact = post(
        &cp,
        &svc,
        "PreCompact",
        tid,
        serde_json::json!({ "session_id": "s1", "trigger": "auto" }),
    )
    .await;
    assert_eq!(compact, serde_json::json!({}));
    let after = post(&cp, &svc, "UserPromptSubmit", tid, prompt()).await;
    golden("prompt_context_first", &after, &root);
}
