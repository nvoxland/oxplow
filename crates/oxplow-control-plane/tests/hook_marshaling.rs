//! HTTP-level tests for the hook surface: auth, envelope routing, and
//! the exact JSON shapes marshaled back to Claude Code (PreToolUse
//! deny bodies, Stop directives). These pin the wire contract the
//! Claude Code plugin depends on — the unit tests in lib.rs cover the
//! helpers, but nothing else exercises `handle_hook` end to end.

#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]
// Test-only crate: terse unwraps are the assertion style here (the
// clippy.toml allow-unwrap-in-tests carve-out doesn't reach helper
// fns in integration-test crates).
#![allow(clippy::unwrap_used)]

mod common;

use common::boot;
use oxplow_tasks::work_item_ref;

use oxplow_app::Services;
use oxplow_control_plane::ControlPlane;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore;
use oxplow_tasks::{Task, TaskActorKind, TaskPriority, TaskStatus};

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

/// The writer edits with nothing tracked: no task, no effort. oxplow
/// never asks for tracked work before an edit (inferred work tracking).
#[tokio::test]
async fn the_writer_edits_without_any_tracked_work() {
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
    assert_eq!(body, serde_json::json!({}));
}

#[tokio::test]
async fn a_stop_is_never_refused_and_keeps_the_final_message() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    seed_in_progress_task(&svc, tid).await;
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

    let resp = post_hook(
        &cp,
        "Stop",
        Some(tid),
        serde_json::json!({ "session_id": "s1", "last_assistant_message": "Did the thing." }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({}));
    let out = oxplow_db::SemanticLayer::new(svc.db.clone())
        .query_sql("SELECT answer FROM v_agent_turn", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&out.rows).unwrap(),
        serde_json::json!([["Did the thing."]])
    );
}

async fn session_of(services: &Services, thread_id: ThreadId) -> oxplow_domain::AgentSessionId {
    services
        .agent_session_store
        .newest_for_thread(thread_id)
        .await
        .unwrap()
        .unwrap()
        .id
}

async fn set_resume_session_id(services: &Services, thread_id: ThreadId, session: &str) {
    let id = session_of(services, thread_id).await;
    let session = session.to_string();
    services
        .db
        .transaction(move |tx| {
            oxplow_db::agent_session_store::set_resume_tx(tx, id, &session, Timestamp::now())
        })
        .await
        .unwrap();
}

async fn resume_session_id(services: &Services, thread_id: ThreadId) -> String {
    services
        .agent_session_store
        .newest_for_thread(thread_id)
        .await
        .unwrap()
        .unwrap()
        .resume_session_id
}

#[tokio::test]
async fn session_end_clear_drops_the_resume_token() {
    // `/clear` ends the session and Claude Code starts a fresh one
    // without any HTTP hook (SessionStart is command-type only), so
    // the resume token would keep pointing at the cleared session
    // until the first prompt. A daemon restart in that window must NOT
    // resurrect the cleared session — SessionEnd(reason=clear) drops
    // the token so the relaunch starts fresh.
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    set_resume_session_id(&svc, tid, "cleared-session").await;
    let resp = post_hook(
        &cp,
        "SessionEnd",
        Some(tid),
        serde_json::json!({ "session_id": "cleared-session", "reason": "clear" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resume_session_id(&svc, tid).await, "");
}

#[tokio::test]
async fn session_end_other_reason_keeps_the_resume_token() {
    // Normal exits (user quit, process end) should still resume — only
    // an explicit clear discards the session.
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    set_resume_session_id(&svc, tid, "keep-me").await;
    let resp = post_hook(
        &cp,
        "SessionEnd",
        Some(tid),
        serde_json::json!({ "session_id": "keep-me", "reason": "other" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resume_session_id(&svc, tid).await, "keep-me");
}

#[tokio::test]
async fn session_end_clear_for_stale_session_keeps_newer_token() {
    // The resume token already moved on to a newer session — a clear
    // of an older one must not wipe it.
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    set_resume_session_id(&svc, tid, "newer-session").await;
    let resp = post_hook(
        &cp,
        "SessionEnd",
        Some(tid),
        serde_json::json!({ "session_id": "old-session", "reason": "clear" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resume_session_id(&svc, tid).await, "newer-session");
}

/// Open another agent session on `thread`, newer than its first.
async fn open_second_session(
    services: &Services,
    thread: ThreadId,
) -> oxplow_domain::AgentSessionId {
    services
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
            thread, "claude",
        ))
        .await
        .unwrap()
        .id
}

/// The agent session each logged event of `ty` was anchored to.
async fn anchored_sessions(
    services: &Services,
    ty: &str,
) -> Vec<Option<oxplow_domain::AgentSessionId>> {
    services
        .event_log_store
        .read_after(0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.envelope.event_type == ty)
        .map(|e| e.envelope.anchors.agent_session_id)
        .collect()
}

/// A hook's `X-Oxplow-Session` names the session it came from, though
/// another session in its thread is newer; one naming no session of the
/// thread is ignored (the thread's session with a turn running takes it).
#[tokio::test]
async fn the_session_header_lands_on_the_hook() {
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let first = session_of(&svc, tid).await;
    open_second_session(&svc, tid).await;
    let post = |session: &str| {
        reqwest::Client::new()
            .post(hook_url(&cp, "UserPromptSubmit"))
            .header("authorization", format!("Bearer {}", cp.hook_token))
            .header("x-oxplow-thread", tid.to_string())
            .header("x-oxplow-session", session.to_string())
            .json(&serde_json::json!({ "prompt": "go" }))
            .send()
    };
    assert_eq!(post(&first.to_string()).await.unwrap().status(), 200);
    assert_eq!(post("ses999").await.unwrap().status(), 200);
    assert_eq!(
        anchored_sessions(&svc, "agent.prompt.submitted").await,
        vec![Some(first), Some(first)]
    );
}

/// An export's `X-Oxplow-Session` anchors its event to that session.
#[tokio::test]
async fn the_session_header_lands_on_an_export() {
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let first = session_of(&svc, tid).await;
    open_second_session(&svc, tid).await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/metrics", cp.otlp_base_url()))
        .header("authorization", format!("Bearer {}", cp.hook_token))
        .header("content-type", "application/x-protobuf")
        .header("x-oxplow-thread", tid.to_string())
        .header("x-oxplow-session", first.to_string())
        .body(otlp_claude_body("claude-opus-4-8", 100, 20))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        anchored_sessions(&svc, "agent.tokens.reported").await,
        vec![Some(first)]
    );
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
async fn post_tool_use_edit_auto_claims_file_on_open_effort() {
    // Child 1 of the claim-first attribution epic: a structured Edit's
    // PostToolUse auto-claims the file onto the thread's OPEN effort in
    // real time, so the agent's touched_files at completion merely
    // confirms/amends rather than enumerating from scratch.
    use oxplow_app::EffortStore as _;
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    // Insert a task and open an effort on the thread.
    let now = Timestamp::from_unix_ms(1);
    let task_id = svc
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(tid),
            parent_id: None,
            title: "ship".into(),
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
    let effort = svc
        .effort_store
        .start(&work_item_ref(task_id), &tid, None)
        .await
        .unwrap();

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
    // The row and the claim are pump reactors on `agent.tool.finished`.
    svc.event_pump.run_once().await.unwrap();

    let files = svc.effort_store.list_files(&effort.id).await.unwrap();
    assert_eq!(files.len(), 1, "the edit should auto-claim one file");
    assert_eq!(files[0].path, "src/x.rs");
}

#[tokio::test]
async fn post_tool_use_is_persisted_as_a_tool_call() {
    let (cp, svc, root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let doc = root.join(".context/usability.md");
    let resp = post_hook(
        &cp,
        "PostToolUse",
        Some(tid),
        serde_json::json!({
            "tool_name": "Read",
            "tool_input": { "file_path": doc.to_string_lossy() },
            "session_id": "s1",
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    // The row and the claim are pump reactors on `agent.tool.finished`.
    svc.event_pump.run_once().await.unwrap();
    let out = oxplow_db::SemanticLayer::new(svc.db.clone())
        .query_sql("SELECT thread_id, path FROM v_context_read", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&out.rows).unwrap(),
        serde_json::json!([[tid.value(), ".context/usability.md"]])
    );
}

#[tokio::test]
async fn prompts_carry_the_efforts_decisions_once_per_session() {
    use oxplow_app::EffortStore as _;
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let now = Timestamp::from_unix_ms(1);
    let task_id = svc
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(tid),
            parent_id: None,
            title: "decide things".into(),
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
    let effort = svc
        .effort_store
        .start(&work_item_ref(task_id), &tid, None)
        .await
        .unwrap();
    svc.db
        .transaction(move |tx| {
            oxplow_db::record_decision_tx(
                tx,
                &oxplow_db::NewDecision {
                    thread_id: tid.value(),
                    work_item: Some(oxplow_tasks::work_item_ref(task_id)),
                    effort_id: Some(effort.id.value()),
                    question: "Storage?".into(),
                    choice: "main DB".into(),
                    alternatives: vec!["attached DB".into()],
                    confidence: "high".into(),
                    why: "cache".into(),
                },
            )
        })
        .await
        .unwrap();

    let context = |body: serde_json::Value| {
        body["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    let prompt = || serde_json::json!({ "prompt": "go", "session_id": "s1" });
    let first = post_hook(&cp, "UserPromptSubmit", Some(tid), prompt())
        .await
        .json()
        .await
        .unwrap();
    assert!(
        context(first).contains("Storage? → main DB"),
        "first prompt of a session carries decisions"
    );
    let second = post_hook(&cp, "UserPromptSubmit", Some(tid), prompt())
        .await
        .json()
        .await
        .unwrap();
    assert!(
        !context(second).contains("Storage?"),
        "not repeated while unchanged"
    );
    // A compaction / resume starts a new context: decisions come back.
    post_hook(
        &cp,
        "SessionStart",
        Some(tid),
        serde_json::json!({ "session_id": "s1", "source": "compact" }),
    )
    .await;
    let third = post_hook(&cp, "UserPromptSubmit", Some(tid), prompt())
        .await
        .json()
        .await
        .unwrap();
    assert!(
        context(third).contains("Storage? → main DB"),
        "re-sent after SessionStart"
    );
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

// ── OTLP metrics receiver (epic tsk22) ──────────────────────────────────────

/// Build an encoded (protobuf) Claude-shaped OTLP metrics export body with one
/// `input` + one `output` `claude_code.token.usage` data point.
fn otlp_claude_body(model: &str, input: i64, output: i64) -> Vec<u8> {
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::metrics::v1::{
        metric, number_data_point, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
    };
    use prost::Message;
    let kv = |k: &str, v: &str| KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(v.into())),
        }),
        ..Default::default()
    };
    let point = |ty: &str, val: i64| NumberDataPoint {
        attributes: vec![kv("type", ty), kv("model", model)],
        value: Some(number_data_point::Value::AsInt(val)),
        ..Default::default()
    };
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: "claude_code.token.usage".into(),
                    data: Some(metric::Data::Sum(Sum {
                        data_points: vec![point("input", input), point("output", output)],
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

/// Build an encoded (protobuf) Codex-shaped OTLP metrics export: a
/// `codex.turn.token_usage` histogram with input/output/reasoning_output points.
fn otlp_codex_body(model: &str, input: f64, output: f64, reasoning: f64) -> Vec<u8> {
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::metrics::v1::{
        metric, Histogram, HistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics,
    };
    use prost::Message;
    let kv = |k: &str, v: &str| KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(v.into())),
        }),
        ..Default::default()
    };
    let hp = |tt: &str, sum: f64| HistogramDataPoint {
        attributes: vec![kv("token_type", tt), kv("model", model)],
        sum: Some(sum),
        ..Default::default()
    };
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: "codex.turn.token_usage".into(),
                    data: Some(metric::Data::Histogram(Histogram {
                        data_points: vec![
                            hp("input", input),
                            hp("output", output),
                            hp("reasoning_output", reasoning),
                        ],
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

/// Build an encoded (protobuf) Codex-shaped OTLP **logs** export: a
/// `codex.sse_event` / `response.completed` record carrying token counts.
fn otlp_codex_logs_body(
    model: &str,
    input: i64,
    cached: i64,
    output: i64,
    reasoning: i64,
) -> Vec<u8> {
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use prost::Message;
    let kv = |k: &str, v: &str| KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(v.into())),
        }),
        ..Default::default()
    };
    let kvi = |k: &str, v: i64| KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::IntValue(v)),
        }),
        ..Default::default()
    };
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![LogRecord {
                    attributes: vec![
                        kv("event.name", "codex.sse_event"),
                        kv("event.kind", "response.completed"),
                        kvi("input_token_count", input),
                        kvi("cached_token_count", cached),
                        kvi("output_token_count", output),
                        kvi("reasoning_token_count", reasoning),
                        kv("model", model),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

/// Post an export as the agent of `thread`, then let the pump count it
/// (the receiver logs `agent.tokens.reported`; a consumer writes facts).
async fn post_otlp(
    cp: &ControlPlane,
    svc: &oxplow_app::Services,
    thread: Option<ThreadId>,
    body: Vec<u8>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}/v1/metrics", cp.otlp_base_url()))
        .header("authorization", format!("Bearer {}", cp.hook_token))
        .header("content-type", "application/x-protobuf")
        .body(body);
    if let Some(t) = thread {
        req = req.header("x-oxplow-thread", t.to_string());
    }
    let resp = req.send().await.unwrap();
    svc.event_pump.run_once().await.unwrap();
    resp
}

#[tokio::test]
async fn otlp_metrics_without_bearer_is_unauthorized() {
    let (cp, _svc, _root, _dir) = boot().await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/metrics", cp.otlp_base_url()))
        .body(otlp_claude_body("claude-opus-4-8", 100, 20))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn otlp_metrics_ingests_token_facts_attributed_by_headers() {
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    let resp = post_otlp(
        &cp,
        &svc,
        Some(tid),
        otlp_claude_body("claude-opus-4-8", 100, 20),
    )
    .await;
    // OTLP success ack is always a 200 (best-effort side-band).
    assert_eq!(resp.status(), 200);

    let measure = svc
        .fact_store
        .get_measure("oxplow.tokens")
        .await
        .unwrap()
        .unwrap();
    let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
    assert_eq!(facts.len(), 2, "input + output token facts landed");
    assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 120.0);
    assert!(facts.iter().all(|f| f.thread_id == Some(tid.value())));
}

#[tokio::test]
async fn otlp_metrics_ingests_codex_histogram_facts() {
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    // input=100, output=20, reasoning_output=30 → output folds to 50, total 150.
    let resp = post_otlp(
        &cp,
        &svc,
        Some(tid),
        otlp_codex_body("gpt-5-codex", 100.0, 20.0, 30.0),
    )
    .await;
    assert_eq!(resp.status(), 200);

    let measure = svc
        .fact_store
        .get_measure("oxplow.tokens")
        .await
        .unwrap()
        .unwrap();
    let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
    assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 150.0);
    assert!(facts
        .iter()
        .all(|f| f.subject_ref.as_deref() == Some("model:gpt-5-codex")));
}

#[tokio::test]
async fn otlp_logs_body_at_metrics_endpoint_ingests_codex_token_facts() {
    // Codex sends its logs (its token source) to the single endpoint we set
    // (/v1/metrics); the ingest path decodes logs when metrics-decode fails.
    let (cp, svc, _root, _dir) = boot().await;
    let tid = seed_thread(&svc, ThreadStatus::Active).await;
    // input 5000 − cached 1000 = 4000 new input; output 200 + reasoning 50 = 250.
    let resp = post_otlp(
        &cp,
        &svc,
        Some(tid),
        otlp_codex_logs_body("gpt-5.5", 5000, 1000, 200, 50),
    )
    .await;
    assert_eq!(resp.status(), 200);

    let measure = svc
        .fact_store
        .get_measure("oxplow.tokens")
        .await
        .unwrap()
        .unwrap();
    let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
    assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 4250.0);
    assert!(facts
        .iter()
        .all(|f| f.subject_ref.as_deref() == Some("model:gpt-5.5")));
}

#[tokio::test]
async fn otlp_metrics_without_attribution_headers_is_dropped_but_acked() {
    let (cp, svc, _root, _dir) = boot().await;
    // No X-Oxplow-Thread/Stream → nothing to attribute to; accept + drop.
    let resp = post_otlp(&cp, &svc, None, otlp_claude_body("m", 100, 20)).await;
    assert_eq!(resp.status(), 200);
    let measure = svc
        .fact_store
        .get_measure("oxplow.tokens")
        .await
        .unwrap()
        .unwrap();
    let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
    assert!(facts.is_empty(), "no facts without attribution headers");
}
