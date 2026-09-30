//! Shared test setup for tests that need a real `Services`.

#![cfg(test)]

use oxplow_domain::refs::build::work_item_ref;
use std::path::Path;
use std::sync::Arc;

use oxplow_db::EffortStore as _;
use oxplow_domain::stores::TaskStore as _;
use oxplow_domain::{
    EffortId, Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus, ThreadId,
};

/// A git repo with one empty commit (stream setup refuses non-git dirs).
pub fn init_git_repo(dir: &Path) {
    let repo = git2::Repository::init(dir).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "test").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    let sig = repo.signature().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
}

/// Stage everything in `dir` and commit it on HEAD; the new sha.
pub fn commit_all(dir: &Path, message: &str) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    idx.write().unwrap();
    let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
    let sig = repo.signature().unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .unwrap()
        .to_string()
}

pub struct EffortFixture {
    pub svc: Arc<crate::Services>,
    /// Keep alive: the project directory.
    pub _dir: tempfile::TempDir,
    pub thread: ThreadId,
    pub task: TaskId,
    pub effort: EffortId,
}

/// In-memory services over a fresh repo with a primary stream, its first
/// thread, an in-progress task on it and an open effort.
pub async fn services_with_effort() -> EffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
    svc.streams.ensure_primary().await.unwrap();
    let thread = ThreadId::new(1);
    let now = oxplow_domain::Timestamp::now();
    let task = svc
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread),
            parent_id: None,
            title: "t".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        })
        .await
        .unwrap();
    let effort = svc
        .effort_store
        .start(&work_item_ref(task), &thread, None)
        .await
        .unwrap()
        .id;
    EffortFixture {
        svc,
        _dir: dir,
        thread,
        task,
        effort,
    }
}
