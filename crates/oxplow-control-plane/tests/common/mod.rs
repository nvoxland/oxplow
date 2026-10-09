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
    let services = Arc::new(Services::in_memory(&root).unwrap());
    // Seed the catalog as boot does — the OTLP token producer gates collection on
    // `measure_has_active_spec` (tsk31), so the token specs must exist.
    services.metrics.seed_catalog().await;
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
