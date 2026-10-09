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
use oxplow_domain::stores::{AgentSessionStore, AgentTurnStore, StreamStore, ThreadStore};
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

/// What a replay leaves: the services, the thread and the agent session
/// it ran as (the tempdir kept alive with them).
struct Replayed {
    svc: std::sync::Arc<oxplow_app::Services>,
    thread: ThreadId,
    session: oxplow_domain::AgentSessionId,
    _cp: oxplow_control_plane::ControlPlane,
    _dir: tempfile::TempDir,
}

/// Replay `name` as a session of `harness` on a fresh thread, then run the
/// suite over it; the edit it made is `edited` (worktree-relative).
async fn replay(name: &str, harness: &str, edited: &str) -> Replayed {
    let r = replayed(name, harness).await;
    let findings = suite(
        &r.svc,
        &Expect {
            harness,
            thread: r.thread,
            edited,
            // A recording holds hooks only; its telemetry isn't replayed.
            tokens: (0, 0),
        },
    )
    .await;
    assert!(findings.is_empty(), "{name}: {findings:#?}");
    r
}

/// Post every hook of `name` as a session of `harness` on a fresh thread.
async fn replayed(name: &str, harness: &str) -> Replayed {
    let (cp, svc, root, dir) = boot().await;
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
    Replayed {
        svc,
        thread: thread.id,
        session: session.id,
        _cp: cp,
        _dir: dir,
    }
}

/// The thread's events of `ty`, oldest first.
async fn events_of(r: &Replayed, ty: &str) -> Vec<serde_json::Value> {
    r.svc
        .event_log_store
        .read_after(0, 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.envelope.anchors.thread_id == Some(r.thread) && e.envelope.event_type == ty)
        .map(|e| e.envelope.payload)
        .collect()
}

/// The harness session the agent session would resume.
async fn resume_of(r: &Replayed) -> String {
    r.svc
        .agent_session_store
        .get(&r.session)
        .await
        .unwrap()
        .unwrap()
        .resume_session_id
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
    let r = replay("opencode/hooks.jsonl", "opencode", "opencode.txt").await;
    // Its Task's child session posted tool hooks under its own id: the
    // session still resumes the parent.
    assert_eq!(resume_of(&r).await, "ses_ee095c454ffep2jtN6LDC21YZJ");
}

/// Claude Code's background subagent, recorded: its own call carries its
/// id, it starts and finishes once, its hand-back opens a turn with no
/// person's prompt, and the session resumes the parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_claude_code_subagent_is_attributed() {
    let r = replayed("claude/subagent.jsonl", "claude").await;
    assert_eq!(events_of(&r, "agent.prompt.submitted").await.len(), 1);
    assert_eq!(events_of(&r, "agent.subagent.started").await.len(), 1);
    assert_eq!(events_of(&r, "agent.subagent.finished").await.len(), 1);
    let bash: Vec<_> = events_of(&r, "agent.tool.finished")
        .await
        .into_iter()
        .filter(|p| p["tool"] == "Bash")
        .collect();
    assert_eq!(bash.len(), 1);
    assert_eq!(bash[0]["subagent"]["id"], "afd849d5d475e378d");
    assert_eq!(bash[0]["subagent"]["kind"], "general-purpose");
    let turns = r
        .svc
        .agent_turn_store
        .list_for_thread(&r.thread, 10)
        .await
        .unwrap();
    let mut prompts: Vec<_> = turns.iter().map(|t| t.prompt.clone()).collect();
    prompts.sort();
    assert_eq!(
        prompts,
        ["", "use a Task subagent to count the lines in claude.txt"]
    );
    assert_eq!(resume_of(&r).await, "56cfaf9b-c90e-4ffd-9689-bd94f89e2426");
}
