//! The program a harness's command hooks run (Codex's `<oxplow> hook
//! <event>`) is the daemon's own executable — what a launch names as
//! oxplow's — so the daemon binary runs `hook <event>`: its stdin posted
//! to the hook route with the session's bearer, the route's answer on its
//! stdout.

#![allow(clippy::unwrap_used)]
#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use oxplow_app::Services;
use oxplow_domain::stores::{AgentSessionStore as _, StreamStore as _, ThreadStore as _};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};

/// A project with one stream, one thread and a Codex session on it; the
/// session's bearer.
async fn codex_session(services: &Services) -> (ThreadId, String) {
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
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
    services.thread_store.upsert(&thread).await.unwrap();
    let session = services
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
            thread.id, "codex",
        ))
        .await
        .unwrap();
    let bearer = services
        .session_auth
        .issue(oxplow_app::session_auth::Principal {
            session: session.id,
            thread: thread.id,
            stream: stream.id,
            harness: "codex".into(),
        });
    (thread.id, bearer)
}

#[tokio::test]
async fn the_daemon_binary_forwards_a_command_hook() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "test"],
        &["config", "user.email", "test@example.com"],
        &["commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    let root = dir.path().canonicalize().unwrap();
    let services = Arc::new(Services::in_memory(&root).unwrap());
    let cp = oxplow_control_plane::spawn(services.clone()).await.unwrap();
    let (thread, bearer) = codex_session(&services).await;

    let base = cp.hook_base_url();
    let out = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_oxplow-daemon"))
            .args(["hook", "UserPromptSubmit"])
            .env("OXPLOW_HOOK_BASE_URL", base)
            .env("OXPLOW_HOOK_TOKEN", bearer)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"hook_event_name":"UserPromptSubmit","prompt":"write codex.txt"}"#)
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(answer.is_object(), "the route's answer: {answer}");
    // The route may answer before its ingest commits (it answers within
    // its budget, ingest finishing on its own): wait for the record.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let prompts = services
            .event_log_store
            .recent("agent.prompt", Some(thread), None, 5)
            .await
            .unwrap();
        if prompts.len() == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the prompt was never ingested"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
