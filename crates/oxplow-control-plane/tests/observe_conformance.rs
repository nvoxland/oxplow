//! The observe conformance suite, live over the fake harness: its launch
//! runs its binary, which posts a scripted session to this control plane's
//! hook route and OTLP receiver, and `oxplow_app::observe_conformance`
//! checks what core recorded. The fake renders its own answer shape and
//! fails on any other, so a pass also shows every answer was the harness's
//! to render (`.context/agent-model.md` "Observe conformance").

#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]
#![allow(clippy::unwrap_used)]

mod common;

use std::process::Command as Proc;
use std::sync::Arc;

use common::boot;

use oxplow_app::observe_conformance::{suite, Expect};
use oxplow_domain::agent::harness::{Endpoints, LaunchInput, LaunchSpec, SessionIds};
use oxplow_domain::agent::text::AgentText;
use oxplow_domain::agent_session::NewAgentSession;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};
use oxplow_harness_fake::{FakeHarness, BIN, EDITED, TOKENS};

/// The fake's binary, built on demand into this test's target dir.
fn fake_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    let bin = dir.join(BIN);
    if !bin.exists() {
        let ok = Proc::new(env!("CARGO"))
            .args(["build", "-q", "-p", "oxplow-harness-fake", "--bin", BIN])
            .env("CARGO_TARGET_DIR", dir.parent().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok, "building {BIN} failed");
    }
    bin
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fake_harness_session_is_recorded_canonically() {
    let (cp, svc, root, _dir) = boot().await;
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
        worktree_path: root.to_string_lossy().into(),
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
    svc.stream_store.upsert(&stream).await.unwrap();
    let thread = Thread {
        id: ThreadId::new(1),
        stream_id: stream.id,
        title: "t".into(),
        status: ThreadStatus::Active,
        sort_index: 0,
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    svc.thread_store.upsert(&thread).await.unwrap();

    // The fake registered as any harness is, and a session of it.
    svc.harnesses.register(Arc::new(FakeHarness));
    let session = svc
        .agent_session_store
        .open(&NewAgentSession::terminal(
            thread.id,
            oxplow_harness_fake::ID,
        ))
        .await
        .unwrap();

    let harness = svc.harnesses.get(oxplow_harness_fake::ID).unwrap();
    let bin = fake_bin().to_string_lossy().into_owned();
    let resolve = move |_: &str| Some(bin.clone());
    let token = common::bearer_for(&svc, session.id).await;
    let identity = vec![
        ("OXPLOW_HOOK_TOKEN".to_string(), token.clone()),
        ("OXPLOW_HOOK_BASE_URL".to_string(), cp.hook_base_url()),
        ("OXPLOW_STREAM_ID".to_string(), stream.id.to_string()),
        ("OXPLOW_THREAD_ID".to_string(), thread.id.to_string()),
        ("OXPLOW_SESSION".to_string(), session.id.to_string()),
    ];
    let launch = harness
        .launch(&LaunchInput {
            session: SessionIds {
                stream: stream.id,
                thread: thread.id,
                session: session.id,
            },
            workspace: &root,
            project_dir: &root,
            endpoints: &Endpoints {
                hook_base_url: cp.hook_base_url(),
                mcp_endpoint_url: cp.mcp_endpoint_url(),
                otlp_base_url: cp.otlp_base_url(),
                hook_token: token.clone(),
            },
            identity_env: &identity,
            system_prompt: None,
            resume: None,
            text: &AgentText::default(),
            config: &serde_json::json!({}),
            oxplow_executable: std::path::Path::new("/bin/false"),
            home: None,
            resolve_program: &resolve,
        })
        .unwrap();
    let LaunchSpec::Pty { command } = launch.spec else {
        panic!("the fake runs in a terminal");
    };
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output(),
    )
    .await
    .expect("the fake's session ends")
    .unwrap();
    assert!(
        out.status.success(),
        "the fake failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let findings = suite(
        &svc,
        &Expect {
            harness: oxplow_harness_fake::ID,
            thread: thread.id,
            edited: EDITED,
            tokens: (TOKENS.0 as u64, TOKENS.1 as u64),
        },
    )
    .await;
    assert!(findings.is_empty(), "{findings:#?}");
}
