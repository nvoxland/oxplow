//! Shared test setup for tests that need a real `Services`.

#![cfg(test)]

use oxplow_tasks::work_item_ref;
use std::path::Path;
use std::sync::Arc;

use oxplow_db::EffortStore as _;
use oxplow_domain::{EffortId, ThreadId};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore as _;
use oxplow_tasks::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};

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

/// Commit what's staged in `dir` on HEAD, as it is; the new sha.
pub fn commit_index(dir: &Path, message: &str) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let mut idx = repo.index().unwrap();
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
    /// The thread's agent session (the project's default agent).
    pub session: oxplow_domain::AgentSessionId,
    pub effort: EffortId,
}

/// An agent session (a Claude terminal) on `thread`, as setup — written to
/// the store, not run as a command, so it leaves no audit row behind.
pub async fn open_session(
    svc: &crate::Services,
    thread: ThreadId,
) -> oxplow_domain::AgentSessionId {
    use oxplow_domain::stores::AgentSessionStore as _;
    svc.agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::of(
            thread,
            oxplow_domain::AgentKind::Claude,
            None,
        ))
        .await
        .unwrap()
        .id
}

/// In-memory services over a fresh repo with a primary stream, its first
/// thread and an open effort linked to nothing — an effort needs no work
/// item (`.context/work-tracking.md`), so a test that isn't about one
/// doesn't make one.
pub async fn services_with_effort() -> EffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
    svc.streams.ensure_primary().await.unwrap();
    let thread = ThreadId::new(1);
    let session = open_session(&svc, thread).await;
    let opened = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::effort::OPEN,
            serde_json::json!({ "thread": thread.to_string() }),
            false,
        )
        .await
        .unwrap();
    let effort = opened.result["effort"]
        .as_str()
        .and_then(|e| e.parse().ok())
        .expect("effort.open names the effort");
    EffortFixture {
        svc,
        _dir: dir,
        thread,
        session,
        effort,
    }
}

/// [`EffortFixture`] whose effort is linked to an in-progress oxplow task
/// on the thread: for a test about tasks.
pub struct TaskEffortFixture {
    pub fx: EffortFixture,
    pub task: TaskId,
}

impl std::ops::Deref for TaskEffortFixture {
    type Target = EffortFixture;

    fn deref(&self) -> &EffortFixture {
        &self.fx
    }
}

/// In-memory services over a fresh repo with a primary stream, its first
/// thread, an in-progress task on it and an open effort linked to it.
pub async fn services_with_task_effort() -> TaskEffortFixture {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
    svc.streams.ensure_primary().await.unwrap();
    let thread = ThreadId::new(1);
    let session = open_session(&svc, thread).await;
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
    restate_task(&svc, task).await;
    let effort = svc
        .effort_store
        .start(&work_item_ref(task), &thread, None)
        .await
        .unwrap()
        .id;
    TaskEffortFixture {
        fx: EffortFixture {
            svc,
            _dir: dir,
            thread,
            session,
            effort,
        },
        task,
    }
}

/// A task filed as a person through `oxplow.work_item.create` with
/// `input` (on the backlog unless it names a `thread`): its id.
pub async fn file_item(svc: &crate::Services, input: serde_json::Value) -> TaskId {
    let out = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::work_item::CREATE,
            input,
            false,
        )
        .await
        .unwrap();
    oxplow_tasks::task_of_work_item_ref(out.result["ref"].as_str().unwrap()).unwrap()
}

/// Restate task `task` in the work-item interface, as its list does after
/// a write: its `work_item.recorded`, logged and projected. For a test
/// that writes the task store directly rather than through the
/// `oxplow.work_item.*` commands.
pub async fn restate_task(svc: &crate::Services, task: TaskId) {
    let record = svc
        .db
        .read(move |tx| oxplow_tasks::record::record_tx(tx, task))
        .await
        .unwrap();
    svc.event_log_store
        .append(oxplow_tasks::provider::recorded(record))
        .await
        .unwrap();
    svc.event_pump.deliver_projections().await.unwrap();
}

/// A new thread on `stream`, made as a person through `oxplow.thread.create`.
pub async fn new_thread(
    svc: &crate::Services,
    stream: oxplow_domain::StreamId,
    title: &str,
) -> oxplow_domain::Thread {
    let out = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::thread::CREATE,
            serde_json::json!({
                "stream": oxplow_domain::refs::build::stream_ref(stream),
                "title": title,
            }),
            false,
        )
        .await
        .unwrap();
    serde_json::from_value(out.result).unwrap()
}
