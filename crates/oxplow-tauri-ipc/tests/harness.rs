// Integration-test code — `unwrap()` is idiomatic here; relax the
// workspace `unwrap_used` guardrail (clippy.toml only exempts unit-test
// modules, not `tests/` helper fns).
#![allow(clippy::unwrap_used)]

//! Shared test harness for `#[tauri::command]` adapters.
//!
//! Builds a `tauri::App` with a mock runtime and a real
//! `oxplow_app::Services` over an in-memory DB, so each command's
//! body can be executed through the same `tauri::State<'_, AppState>`
//! plumbing the production shell uses. The aim is to cover the
//! argument-shape + error-mapping seam in
//! `crates/oxplow-tauri-ipc/src/commands/*` — store logic is
//! exercised in `oxplow-app` integration tests.

#![allow(dead_code)]

use oxplow_app::Services;
use oxplow_tauri_ipc::AppState;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};
use tempfile::TempDir;

pub struct TestApp {
    pub _tmp: TempDir,
    pub state: AppState,
    pub app: App<MockRuntime>,
}

impl TestApp {
    pub async fn build() -> Self {
        let tmp = TempDir::new().expect("tmp project dir");
        // Services::in_memory now calls ensure_primary, which requires
        // the project_dir to be a git repo. Init one with a single
        // empty commit so the tests have a valid stream context.
        let repo = git2::Repository::init(tmp.path()).expect("git init");
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let services = Services::in_memory(tmp.path())
            .await
            .expect("services in_memory");
        let state: AppState = services;
        let app = mock_builder()
            .manage(state.clone())
            .build(mock_context(noop_assets()))
            .expect("mock app build");
        TestApp {
            _tmp: tmp,
            state,
            app,
        }
    }

    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}
