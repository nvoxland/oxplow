//! Stream + worktree lifecycle.
//!
//! Encodes the primary-vs-worktree invariant from
//! `.context/architecture.md`: exactly one primary stream per project,
//! everything else is a worktree at `<parent>/<basename>-<slug>/` —
//! a sibling of the main repo (see `.context/architecture.md`).
//!
//! Composes `oxplow-git` (for actual `git worktree add`) and the
//! `StreamStore` trait from `oxplow-domain` for persistence.

pub mod thread_service;
pub use thread_service::{ThreadError, ThreadService};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;
use tracing::info;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{DomainError, Stream, StreamId, StreamKind, Timestamp};
use oxplow_git::{detect_current_branch, ensure_worktree, is_git_repo, is_git_worktree};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("workspace is not a git repo: {0}")]
    NotARepo(PathBuf),
    #[error("workspace is a secondary git worktree (refusing to start): {0}")]
    InWorktree(PathBuf),
    #[error("primary stream already exists")]
    PrimaryExists,
    #[error("primary stream missing")]
    PrimaryMissing,
    #[error("worktree slug \"{0}\" already exists")]
    DuplicateWorktreeSlug(String),
    #[error("git: {0}")]
    Git(#[from] oxplow_git::EnsureWorktreeError),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

/// Configuration for stream-management.
#[derive(Debug, Clone)]
pub struct WorkspaceLayout {
    /// Project root — the daemon's start directory.
    pub project_dir: PathBuf,
    /// Parent directory of `project_dir`; new worktrees land here as
    /// `<project_basename>-<slug>/` siblings of the main repo. Falls
    /// back to `project_dir` itself if the project has no parent
    /// (e.g. `/`), in which case `git worktree add` will surface the
    /// error.
    pub worktrees_root: PathBuf,
    /// Project basename (the leaf segment of `project_dir`). Used to
    /// namespace sibling worktree directories so two projects sharing
    /// the same parent don't collide on a slug.
    pub project_slug_prefix: String,
}

impl WorkspaceLayout {
    pub fn for_project(project_dir: impl Into<PathBuf>) -> Self {
        let project_dir = project_dir.into();
        let worktrees_root = project_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| project_dir.clone());
        let project_slug_prefix = project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "oxplow".into());
        Self {
            project_dir,
            worktrees_root,
            project_slug_prefix,
        }
    }

    /// Resolve the on-disk path for a new worktree of the given slug.
    /// Sibling pattern: `<parent>/<project_basename>-<slug>/`.
    pub fn worktree_path_for(&self, slug: &str) -> PathBuf {
        self.worktrees_root
            .join(format!("{}-{}", self.project_slug_prefix, slug))
    }
}

/// Top-level service. Cheap to clone — internals are `Arc`'d.
#[derive(Clone)]
pub struct StreamService {
    layout: WorkspaceLayout,
    streams: Arc<dyn StreamStore>,
    threads: Arc<dyn oxplow_domain::stores::ThreadStore>,
}

/// Default title applied to the auto-generated thread that every
/// new stream gets. The model invariant "every stream has ≥1 thread"
/// is enforced by `StreamService` itself — every stream-creation path
/// calls `seed_default_thread` after upserting the stream.
const DEFAULT_THREAD_TITLE: &str = "Thread";

impl StreamService {
    pub fn new(
        layout: WorkspaceLayout,
        streams: Arc<dyn StreamStore>,
        threads: Arc<dyn oxplow_domain::stores::ThreadStore>,
    ) -> Self {
        Self {
            layout,
            streams,
            threads,
        }
    }

    /// Insert the auto-created `"Thread"` row that every fresh stream
    /// owns. Idempotent: skips when threads already exist for the
    /// stream (e.g. worktree adoption may resolve to a previously
    /// seeded stream). Failure is logged but doesn't fail the stream
    /// creation — a thread-less stream is still navigable, just
    /// awkwardly empty.
    async fn seed_default_thread(&self, stream_id: &StreamId) {
        let existing = match self.threads.list_for_stream(stream_id).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(stream_id = %stream_id, error = %e, "list_for_stream failed during seed");
                return;
            }
        };
        if !existing.is_empty() {
            return;
        }
        let now = Timestamp::now();
        let thread = oxplow_domain::Thread {
            id: oxplow_domain::ThreadId::placeholder(),
            stream_id: *stream_id,
            title: DEFAULT_THREAD_TITLE.into(),
            status: oxplow_domain::ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        if let Err(e) = self.threads.upsert(&thread).await {
            tracing::warn!(stream_id = %stream_id, error = %e, "default thread create failed");
        }
    }

    /// Validate the workspace before doing anything else: it must be
    /// a git repo and must NOT be a secondary worktree.
    pub fn validate_workspace(&self) -> Result<(), SessionError> {
        if !is_git_repo(&self.layout.project_dir) {
            return Err(SessionError::NotARepo(self.layout.project_dir.clone()));
        }
        if is_git_worktree(&self.layout.project_dir) {
            return Err(SessionError::InWorktree(self.layout.project_dir.clone()));
        }
        Ok(())
    }

    /// Idempotent: ensures a primary stream exists for this project.
    /// Reuses the existing one if present.
    pub async fn ensure_primary(&self) -> Result<Stream, SessionError> {
        self.validate_workspace()?;
        if let Some(existing) = self.streams.primary().await? {
            return Ok(existing);
        }
        let branch =
            detect_current_branch(&self.layout.project_dir).unwrap_or_else(|| "HEAD".to_string());
        let now = Timestamp::now();
        let title = self
            .layout
            .project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "primary".into());
        let mut stream = Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Primary,
            title,
            branch: branch.clone(),
            branch_ref: format!("refs/heads/{branch}"),
            branch_source: branch,
            worktree_path: self.layout.project_dir.to_string_lossy().into_owned(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        stream.id = self.streams.upsert(&stream).await?;
        self.seed_default_thread(&stream.id).await;
        info!(stream_id = %stream.id, "primary stream created");
        Ok(stream)
    }

    /// Create a worktree stream. New worktrees are placed as siblings
    /// of the main repo at `<parent>/<project_basename>-<slug>/`. The
    /// slug is fixed at creation and never changes; `branch_source`
    /// is the branch to fork from (e.g. `main`).
    pub async fn create_worktree(
        &self,
        slug: &str,
        title: impl Into<String>,
        branch: impl Into<String>,
        branch_source: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        self.validate_workspace()?;
        // Ensure primary exists so the layout invariant holds.
        let _primary = self
            .streams
            .primary()
            .await?
            .ok_or(SessionError::PrimaryMissing)?;

        let branch = branch.into();
        let branch_source = branch_source.into();
        let title = title.into();

        let worktree_path = self.layout.worktree_path_for(slug);

        // Reject duplicate slug — not on the path, but on a stream
        // already pointing at the same path. (We can't easily query the
        // store by path without a new method, so we check the cheap
        // duplicate case: anything at the path means we punt.)
        if worktree_path.exists() {
            // Scan existing streams for the path; if found, return that
            // stream (idempotent ensure semantics).
            for existing in self.streams.list().await? {
                if existing.worktree_path == worktree_path.to_string_lossy() {
                    return Ok(existing);
                }
            }
            return Err(SessionError::DuplicateWorktreeSlug(slug.to_string()));
        }

        ensure_worktree(
            &self.layout.project_dir,
            &worktree_path,
            &branch,
            &branch_source,
        )?;

        let now = Timestamp::now();
        let mut stream = Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Worktree,
            title,
            branch: branch.clone(),
            branch_ref: format!("refs/heads/{branch}"),
            branch_source,
            worktree_path: worktree_path.to_string_lossy().into_owned(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        stream.id = self.streams.upsert(&stream).await?;
        self.seed_default_thread(&stream.id).await;
        info!(stream_id = %stream.id, slug, "worktree stream created");
        Ok(stream)
    }

    /// Register an existing on-disk worktree as a new oxplow stream.
    ///
    /// The path must already exist as a `git worktree` of the
    /// project's primary repo (the renderer's `listAdoptableWorktrees`
    /// is the source of valid candidates). The branch and detected
    /// current branch are read from the worktree itself; we don't
    /// `git worktree add` here. Idempotent: if a stream already
    /// points at the path we return it unchanged.
    pub async fn adopt_worktree(
        &self,
        worktree_path: PathBuf,
        title: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        self.validate_workspace()?;
        let _primary = self
            .streams
            .primary()
            .await?
            .ok_or(SessionError::PrimaryMissing)?;

        // Idempotent: if a stream already tracks this path, return
        // it. Path comparison is string-based (matches what gets
        // persisted in stream_store).
        let path_str = worktree_path.to_string_lossy().into_owned();
        for existing in self.streams.list().await? {
            if existing.worktree_path == path_str {
                return Ok(existing);
            }
        }

        // Read the branch from the worktree's HEAD. If the worktree
        // is detached we still record it so the user can fix that on
        // their own (the rest of oxplow tolerates a missing branch).
        let branch = detect_current_branch(&worktree_path).unwrap_or_else(|| "HEAD".to_string());

        let title = title.into();
        let now = Timestamp::now();
        let mut stream = Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Worktree,
            title,
            branch: branch.clone(),
            branch_ref: format!("refs/heads/{branch}"),
            branch_source: branch,
            worktree_path: path_str,
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        stream.id = self.streams.upsert(&stream).await?;
        self.seed_default_thread(&stream.id).await;
        info!(stream_id = %stream.id, path = %worktree_path.display(), "adopted existing worktree");
        Ok(stream)
    }

    /// List all streams ordered with primary first.
    pub async fn list_streams(&self) -> Result<Vec<Stream>, SessionError> {
        Ok(self.streams.list().await?)
    }

    /// Get the runtime-state-pinned current stream (None if nothing
    /// has been selected yet — caller typically falls back to primary).
    pub async fn current(&self) -> Result<Option<Stream>, SessionError> {
        match self.streams.current_id().await? {
            None => Ok(None),
            Some(id) => Ok(self.streams.get(&id).await?),
        }
    }

    /// Set the current stream pointer. `None` clears the pointer.
    pub async fn set_current(&self, id: Option<&StreamId>) -> Result<(), SessionError> {
        if let Some(id) = id {
            // Validate the target exists before writing the pointer.
            self.streams
                .get(id)
                .await?
                .ok_or(SessionError::Storage(DomainError::NotFound))?;
        }
        self.streams.set_current(id).await?;
        Ok(())
    }

    pub async fn rename(
        &self,
        id: &StreamId,
        title: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        s.title = title.into();
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Set per-stream pane targets. Either field can be empty to clear.
    pub async fn set_panes(
        &self,
        id: &StreamId,
        working: Option<String>,
        talking: Option<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        if let Some(w) = working {
            s.working_pane = w;
        }
        if let Some(t) = talking {
            s.talking_pane = t;
        }
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Update the stream's recorded branch + branch_ref to reflect a
    /// successful checkout. Doesn't itself run the checkout — the
    /// caller (typically the git-ops layer) does that and then asks
    /// us to record the new state.
    pub async fn record_branch_checkout(
        &self,
        id: &StreamId,
        branch: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        let branch = branch.into();
        s.branch = branch.clone();
        s.branch_ref = format!("refs/heads/{branch}");
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Archive a stream and every thread under it. Soft-delete via
    /// `archived_at` — the rows stay in the DB so closed efforts,
    /// snapshots, and page_visit attribution don't dangle, but the
    /// rail and thread queries filter them out. If `delete_worktree`
    /// is true and the stream is a worktree (not primary), the
    /// on-disk worktree is also `git worktree remove --force`'d.
    /// Primary streams cannot be archived.
    pub async fn archive_stream(
        &self,
        id: &StreamId,
        delete_worktree: bool,
    ) -> Result<(), SessionError> {
        let stream = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        if stream.kind == StreamKind::Primary {
            return Err(SessionError::Storage(DomainError::Invariant(
                "cannot archive primary stream".into(),
            )));
        }
        // Archive every thread first so the rail+work surfaces drop
        // them in lockstep with the stream.
        let threads = self.threads.list_for_stream(id).await?;
        for t in threads {
            self.threads.archive(&t.id).await?;
        }
        self.streams.archive(id).await?;
        // Optional on-disk teardown. Best-effort: if git refuses (dirty
        // tree, locked, missing) the row is already archived, so the
        // rail no longer shows it; the user can clean up the directory
        // manually.
        if delete_worktree {
            let path = Path::new(&stream.worktree_path);
            if path.exists() {
                let _ = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&self.layout.project_dir)
                    .arg("worktree")
                    .arg("remove")
                    .arg("--force")
                    .arg(path)
                    .output();
            }
            // Always run `git worktree prune` after a remove. If
            // `remove` failed (e.g. directory was already gone, or
            // the worktree was locked) git keeps a stale admin entry
            // in `.git/worktrees/`; prune is what actually clears it
            // so subsequent `git worktree add` won't trip on the
            // ghost.
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.layout.project_dir)
                .arg("worktree")
                .arg("prune")
                .output();
        }
        Ok(())
    }

    /// Delete a stream. The primary cannot be deleted — that's the
    /// project itself and would leave the workspace in an incoherent
    /// state.
    pub async fn delete_stream(&self, id: &StreamId) -> Result<(), SessionError> {
        let stream = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        if stream.kind == StreamKind::Primary {
            return Err(SessionError::Storage(DomainError::Invariant(
                "cannot delete primary stream".into(),
            )));
        }
        // For worktree streams, tear down the on-disk worktree first
        // (best-effort — if `git worktree remove` fails, the user
        // can still see the row in the DB and clean up manually).
        let path = Path::new(&stream.worktree_path);
        if path.exists() {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.layout.project_dir)
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(path)
                .output();
        }
        self.streams.delete(id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use std::process::Command;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) {
        // Pin the initial branch to "main" via init options so the
        // tests don't depend on the system-wide init.defaultBranch
        // (CI runners often default to "master").
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(dir, &opts).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        config.set_str("init.defaultBranch", "main").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    /// Wraps the project dir inside a parent tempdir so sibling
    /// worktrees (which now land at `<parent>/<basename>-<slug>/`)
    /// get cleaned up when the parent drops.
    struct TestEnv {
        _parent: tempfile::TempDir,
        project: PathBuf,
    }

    impl TestEnv {
        #[allow(dead_code)]
        fn project_path(&self) -> &Path {
            &self.project
        }

        /// Resolve where a worktree with `slug` will land, given the
        /// `<parent>/<basename>-<slug>/` sibling pattern.
        fn worktree_path(&self, slug: &str) -> PathBuf {
            let basename = self
                .project
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".into());
            self.project
                .parent()
                .unwrap()
                .join(format!("{basename}-{slug}"))
        }
    }

    fn make_service() -> (StreamService, TestEnv) {
        let parent = tempdir().unwrap();
        let project = parent.path().join("project");
        std::fs::create_dir(&project).unwrap();
        init_repo(&project);
        let layout = WorkspaceLayout::for_project(&project);
        let db = Database::in_memory();
        let streams = Arc::new(SqliteStreamStore::new(db.clone()));
        let threads = Arc::new(SqliteThreadStore::new(db));
        let service = StreamService::new(layout, streams, threads);
        (
            service,
            TestEnv {
                _parent: parent,
                project,
            },
        )
    }

    #[tokio::test]
    async fn validate_workspace_passes_for_repo() {
        let (svc, _dir) = make_service();
        svc.validate_workspace().unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_non_repo() {
        let project = tempdir().unwrap();
        let layout = WorkspaceLayout::for_project(project.path());
        let db = Database::in_memory();
        let streams = Arc::new(SqliteStreamStore::new(db.clone()));
        let threads = Arc::new(SqliteThreadStore::new(db));
        let svc = StreamService::new(layout, streams, threads);
        let err = svc.validate_workspace().unwrap_err();
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

    /// Suppress unused warning when the test for `Command` is gone.
    #[allow(dead_code)]
    fn _silence(_: Command) {}
}
