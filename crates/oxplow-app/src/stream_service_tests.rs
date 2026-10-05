//! `StreamService` (`oxplow-session`) over the git provider: its streams
//! live in real working copies, so its tests run here, beside the
//! provider, with a real repository.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
use oxplow_domain::stores::ThreadStore as _;
use oxplow_domain::{AgentKind, DomainError, StreamKind};
use oxplow_session::{SessionError, StreamService, WorkspaceLayout};
use tempfile::tempdir;

use crate::vcs::GitProvider;

fn init_repo(dir: &Path) {
    // Pin the initial branch to "main" so the tests don't depend on the
    // system-wide init.defaultBranch (CI runners often default to
    // "master").
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = git2::Repository::init_opts(dir, &opts).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "test").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    let sig = repo.signature().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
}

/// Wraps the project dir inside a parent tempdir so sibling worktrees
/// (which land at `<parent>/<basename>-<slug>/`) are cleaned up when the
/// parent drops.
struct TestEnv {
    _parent: tempfile::TempDir,
    project: PathBuf,
}

impl TestEnv {
    /// Where a worktree with `slug` lands.
    fn worktree_path(&self, slug: &str) -> PathBuf {
        let basename = self.project.file_name().unwrap().to_string_lossy();
        self.project
            .parent()
            .unwrap()
            .join(format!("{basename}-{slug}"))
    }
}

fn service(project: &Path) -> StreamService {
    service_running(project, AgentKind::Claude, None)
}

/// A service whose new threads run `agent` (and `acp_agent`).
fn service_running(project: &Path, agent: AgentKind, acp_agent: Option<&str>) -> StreamService {
    let db = Database::in_memory();
    let acp_agent = acp_agent.map(str::to_string);
    StreamService::new(
        WorkspaceLayout::for_project(project),
        Arc::new(GitProvider),
        Arc::new(SqliteStreamStore::new(db.clone())),
        Arc::new(SqliteThreadStore::new(db)),
        Arc::new(move || (agent, acp_agent.clone())),
    )
}

/// tsk970: a stream's seeded thread runs the project's default agent, not
/// always Claude.
#[tokio::test]
async fn the_seeded_thread_runs_the_projects_default_agent() {
    let parent = tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir(&project).unwrap();
    init_repo(&project);
    let db = Database::in_memory();
    let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
    let svc = StreamService::new(
        WorkspaceLayout::for_project(&project),
        Arc::new(GitProvider),
        Arc::new(SqliteStreamStore::new(db)),
        thread_store.clone(),
        Arc::new(|| (AgentKind::Acp, Some("fake".to_string()))),
    );
    let primary = svc.ensure_primary().await.unwrap();
    let threads = thread_store.list_for_stream(&primary.id).await.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(
        (threads[0].agent, threads[0].acp_agent.as_deref()),
        (AgentKind::Acp, Some("fake"))
    );
}

fn make_service() -> (StreamService, TestEnv) {
    let parent = tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir(&project).unwrap();
    init_repo(&project);
    (
        service(&project),
        TestEnv {
            _parent: parent,
            project,
        },
    )
}

#[tokio::test]
async fn validate_rejects_a_secondary_worktree() {
    let (svc, dir) = make_service();
    svc.ensure_primary().await.unwrap();
    svc.create_worktree("side", "Side", "side", "main")
        .await
        .unwrap();
    let err = service(&dir.worktree_path("side"))
        .validate_workspace()
        .await
        .unwrap_err();
    assert!(matches!(err, SessionError::InWorktree(_)), "{err:?}");
}

#[tokio::test]
async fn validate_workspace_passes_for_repo() {
    let (svc, _dir) = make_service();
    svc.validate_workspace().await.unwrap();
}

#[tokio::test]
async fn validate_rejects_non_repo() {
    let project = tempdir().unwrap();
    let svc = service(project.path());
    let err = svc.validate_workspace().await.unwrap_err();
    assert!(matches!(err, SessionError::NotARepo(_)));
}

#[tokio::test]
async fn ensure_primary_creates_then_idempotent() {
    let (svc, _dir) = make_service();
    let first = svc.ensure_primary().await.unwrap();
    assert_eq!(first.kind, StreamKind::Primary);
    let second = svc.ensure_primary().await.unwrap();
    assert_eq!(first.id, second.id);
}

#[tokio::test]
async fn create_worktree_makes_worktree_and_row() {
    let (svc, dir) = make_service();
    let _primary = svc.ensure_primary().await.unwrap();
    let stream = svc
        .create_worktree("feature-1", "Feature 1", "feature-1", "main")
        .await
        .unwrap();
    assert_eq!(stream.kind, StreamKind::Worktree);
    let path = dir.worktree_path("feature-1");
    assert!(path.exists(), "worktree dir should exist at {path:?}");
}

#[tokio::test]
async fn create_worktree_requires_primary() {
    let (svc, _dir) = make_service();
    let err = svc
        .create_worktree("feature-1", "Feature 1", "feature-1", "main")
        .await
        .unwrap_err();
    assert!(matches!(err, SessionError::PrimaryMissing));
}

#[tokio::test]
async fn delete_primary_rejected() {
    let (svc, _dir) = make_service();
    let primary = svc.ensure_primary().await.unwrap();
    let err = svc.delete_stream(&primary.id).await.unwrap_err();
    assert!(matches!(
        err,
        SessionError::Storage(DomainError::Invariant(_))
    ));
}

#[tokio::test]
async fn delete_worktree_removes_row_and_dir() {
    let (svc, dir) = make_service();
    svc.ensure_primary().await.unwrap();
    let stream = svc
        .create_worktree("feature-rm", "Feature", "feature-rm", "main")
        .await
        .unwrap();
    let path = dir.worktree_path("feature-rm");
    assert!(path.exists());
    svc.delete_stream(&stream.id).await.unwrap();
    // git worktree remove deletes the dir; the row is gone.
    assert!(svc
        .list_streams()
        .await
        .unwrap()
        .iter()
        .all(|s| s.id != stream.id));
    // best-effort dir removal — verify
    assert!(!path.exists(), "worktree dir should be removed");
}

#[tokio::test]
async fn archive_primary_rejected() {
    let (svc, _dir) = make_service();
    let primary = svc.ensure_primary().await.unwrap();
    let err = svc.archive_stream(&primary.id, false).await.unwrap_err();
    assert!(matches!(
        err,
        SessionError::Storage(DomainError::Invariant(_))
    ));
}

#[tokio::test]
async fn archive_drops_stream_from_list_but_keeps_dir() {
    let (svc, dir) = make_service();
    svc.ensure_primary().await.unwrap();
    let stream = svc
        .create_worktree("feat-archive", "Feat", "feat-archive", "main")
        .await
        .unwrap();
    let path = dir.worktree_path("feat-archive");
    assert!(path.exists());
    // delete_worktree=false: row is archived, dir remains on disk.
    svc.archive_stream(&stream.id, false).await.unwrap();
    assert!(svc
        .list_streams()
        .await
        .unwrap()
        .iter()
        .all(|s| s.id != stream.id));
    assert!(
        path.exists(),
        "worktree dir should remain when delete_worktree=false"
    );
}

#[tokio::test]
async fn archive_with_delete_worktree_removes_dir() {
    let (svc, dir) = make_service();
    svc.ensure_primary().await.unwrap();
    let stream = svc
        .create_worktree("feat-arch-del", "Feat", "feat-arch-del", "main")
        .await
        .unwrap();
    let path = dir.worktree_path("feat-arch-del");
    assert!(path.exists());
    svc.archive_stream(&stream.id, true).await.unwrap();
    assert!(
        !path.exists(),
        "worktree dir should be pruned when delete_worktree=true"
    );
}

#[tokio::test]
async fn list_orders_primary_first() {
    let (svc, _dir) = make_service();
    svc.ensure_primary().await.unwrap();
    svc.create_worktree("a", "A", "a", "main").await.unwrap();
    svc.create_worktree("b", "B", "b", "main").await.unwrap();
    let list = svc.list_streams().await.unwrap();
    assert_eq!(list[0].kind, StreamKind::Primary);
    assert!(list[1..].iter().all(|s| s.kind == StreamKind::Worktree));
}
