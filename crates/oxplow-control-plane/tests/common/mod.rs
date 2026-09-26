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
