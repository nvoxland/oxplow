//! Shared boot for the control-plane integration tests: a real git repo,
//! in-memory services with the metric catalog seeded, and a live server.

// Terse unwraps are the assertion style in test crates.
#![allow(clippy::unwrap_used)]

use std::process::Command;
use std::sync::Arc;

use oxplow_app::Services;
use oxplow_control_plane::{spawn, ControlPlane};

pub async fn boot() -> (
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
    let services = Services::in_memory(&root).await.unwrap();
    // Seed the catalog as boot does — the OTLP token producer gates collection on
    // `measure_has_active_spec` (tsk31), so the token specs must exist.
    let cp = spawn(services.clone()).await.unwrap();
    (cp, services, root, dir)
}

/// A bearer for `thread`'s newest agent session (one opened when it has
/// none), as a launch mints it: what a hook, export or MCP call from that
/// session carries. Minting again retires the session's earlier bearer.
#[allow(dead_code, reason = "not every test crate posts as an agent")]
#[allow(
    clippy::disallowed_methods,
    reason = "a test seeds the session through its store"
)]
pub async fn bearer(services: &Services, thread: oxplow_domain::ThreadId) -> String {
    use oxplow_domain::stores::AgentSessionStore as _;
    let session = match services
        .agent_session_store
        .newest_for_thread(thread)
        .await
        .unwrap()
    {
        Some(s) => s,
        None => services
            .agent_session_store
            .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
                thread, "claude",
            ))
            .await
            .unwrap(),
    };
    bearer_for(services, session.id).await
}

/// A bearer for agent session `session`.
#[allow(dead_code, reason = "not every test crate posts as an agent")]
pub async fn bearer_for(services: &Services, session: oxplow_domain::AgentSessionId) -> String {
    use oxplow_domain::stores::{AgentSessionStore as _, ThreadStore as _};
    let row = services
        .agent_session_store
        .get(&session)
        .await
        .unwrap()
        .expect("the session exists");
    let thread = services
        .thread_store
        .get(&row.thread_id)
        .await
        .unwrap()
        .expect("its thread exists");
    services
        .session_auth
        .issue(oxplow_app::session_auth::Principal {
            session,
            thread: thread.id,
            stream: thread.stream_id,
            harness: row.harness.clone(),
        })
}

/// A worktree stream (id 2) beside the project — `<root>-wt`, as
/// stream creation makes one — whose tree no thread of another stream may
/// edit. Returns its worktree.
#[allow(dead_code, reason = "not every test crate edits across streams")]
#[allow(
    clippy::disallowed_methods,
    reason = "a test seeds the stream through its store"
)]
pub async fn seed_worktree_stream(
    services: &Services,
    root: &std::path::Path,
) -> std::path::PathBuf {
    use oxplow_domain::stores::StreamStore as _;
    let now = oxplow_domain::Timestamp::from_unix_ms(1);
    let tree = std::path::PathBuf::from(format!("{}-wt", root.display()));
    let stream = oxplow_domain::Stream {
        id: oxplow_domain::StreamId::new(2),
        kind: oxplow_domain::StreamKind::Worktree,
        title: "wt".into(),
        branch: "feat".into(),
        branch_ref: "refs/heads/feat".into(),
        branch_source: "main".into(),
        worktree_path: tree.to_string_lossy().into(),
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
    tree
}
