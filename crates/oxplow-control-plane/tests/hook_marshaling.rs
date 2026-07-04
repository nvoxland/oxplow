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
    // Seed the catalog as boot does — the OTLP token producer gates collection on
    // `measure_has_active_spec` (tsk31), so the token specs must exist.
    services.metrics.seed_catalog().await;
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

async fn set_resume_session_id(services: &Services, thread_id: ThreadId, session: &str) {
    let mut thread = services
        .thread_store
        .get(&thread_id)
        .await
        .unwrap()
        .unwrap();
    thread.resume_session_id = session.to_string();
    services.thread_store.upsert(&thread).await.unwrap();
}

async fn resume_session_id(services: &Services, thread_id: ThreadId) -> String {
    services
        .thread_store
        .get(&thread_id)
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
    use oxplow_app::TaskEffortStore as _;
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
    let effort = svc.effort_store.start(task_id, &tid, None).await.unwrap();

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

    let files = svc.effort_store.list_files(&effort.id).await.unwrap();
    assert_eq!(files.len(), 1, "the edit should auto-claim one file");
    assert_eq!(files[0].path, "src/x.rs");
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

async fn post_otlp(
    cp: &ControlPlane,
    thread: Option<ThreadId>,
    stream: Option<StreamId>,
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
    if let Some(s) = stream {
        req = req.header("x-oxplow-stream", s.to_string());
    }
    req.send().await.unwrap()
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
        Some(tid),
        Some(StreamId::new(1)),
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
        Some(tid),
        Some(StreamId::new(1)),
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
        Some(tid),
        Some(StreamId::new(1)),
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
    let resp = post_otlp(&cp, None, None, otlp_claude_body("m", 100, 20)).await;
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
