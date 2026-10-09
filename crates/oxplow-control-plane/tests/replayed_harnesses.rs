//! The real harnesses' recorded sessions, replayed: each fixture under
//! `crates/oxplow-harnesses/fixtures/<harness>/` is one session of Claude
//! Code, Codex or opencode as `OXPLOW_HOOK_DEBUG` recorded it (paths
//! written `<ROOT>`, `<HOME>`, `<TMP>`). Every hook is posted, in order,
//! to a live control plane's hook route as a session of that harness, so
//! the harness's own mapping and answers run as they do for a real agent,
//! and `oxplow_app::observe_conformance` checks what core recorded
//! (`.context/agent-model.md` "Observe conformance").

#![allow(
    clippy::disallowed_methods,
    reason = "a test seeds the database through its stores"
)]
#![allow(clippy::unwrap_used)]

mod common;

use common::boot;

use oxplow_app::observe_conformance::{suite, Expect};
use oxplow_domain::agent_session::NewAgentSession;
use oxplow_domain::stores::{AgentSessionStore, StreamStore, ThreadStore};
use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};

/// One recorded hook: its event and its body as the agent sent it.
#[derive(serde::Deserialize)]
struct Recorded {
    event: String,
    payload: serde_json::Value,
}

/// The fixture `name`, its placeholders filled for `root`.
fn fixture(name: &str, root: &std::path::Path) -> Vec<Recorded> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxplow-harnesses/fixtures")
        .join(name);
    let root = root.to_string_lossy();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let l = l
                .replace("<ROOT>", &root)
                .replace("<HOME>", "/home/someone")
                .replace("<TMP>", "/tmp/someone");
            serde_json::from_str(&l).unwrap()
        })
        .collect()
}

/// Replay `name` as a session of `harness` on a fresh thread, then run the
/// suite over it; the edit it made is `edited` (worktree-relative).
async fn replay(name: &str, harness: &str, edited: &str) {
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
    let session = svc
        .agent_session_store
        .open(&NewAgentSession::terminal(thread.id, harness))
        .await
        .unwrap();
    let token = common::bearer_for(&svc, session.id).await;

    let client = reqwest::Client::new();
    for hook in fixture(name, &root) {
        let resp = client
            .post(format!("{}/{}", cp.hook_base_url(), hook.event))
            .bearer_auth(&token)
            .json(&hook.payload)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "{} {}", hook.event, hook.payload);
    }

    let findings = suite(
        &svc,
        &Expect {
            harness,
            thread: thread.id,
            edited,
            // A recording holds hooks only; its telemetry isn't replayed.
            tokens: (0, 0),
        },
    )
    .await;
    assert!(findings.is_empty(), "{name}: {findings:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_claude_code_session_is_recorded_canonically() {
    replay("claude/hooks.jsonl", "claude", "claude.txt").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_codex_session_is_recorded_canonically() {
    replay("codex/hooks.jsonl", "codex", "codex.txt").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_opencode_session_is_recorded_canonically() {
    replay("opencode/hooks.jsonl", "opencode", "opencode.txt").await;
}
