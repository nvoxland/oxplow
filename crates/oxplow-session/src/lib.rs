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
use oxplow_domain::vcs::{Checkout, Vcs, VcsError};
use oxplow_domain::{DomainError, Stream, StreamId, StreamKind, Timestamp};

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
    #[error("vcs: {0}")]
    Vcs(#[from] VcsError),
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
    vcs: Arc<dyn Vcs>,
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
        vcs: Arc<dyn Vcs>,
        streams: Arc<dyn StreamStore>,
        threads: Arc<dyn oxplow_domain::stores::ThreadStore>,
    ) -> Self {
        Self {
            layout,
            vcs,
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
        // A thread needs no agent: the person opens its sessions.
        let thread =
            oxplow_domain::Thread::seed(*stream_id, DEFAULT_THREAD_TITLE, Timestamp::now());
        if let Err(e) = self.threads.upsert(&thread).await {
            tracing::warn!(stream_id = %stream_id, error = %e, "default thread create failed");
        }
    }

    /// Validate the workspace before doing anything else: it must be
    /// under version control and must NOT be a secondary working copy.
    pub async fn validate_workspace(&self) -> Result<(), SessionError> {
        match self.vcs.detect(&self.layout.project_dir).await {
            None => Err(SessionError::NotARepo(self.layout.project_dir.clone())),
            Some(Checkout::Secondary) => {
                Err(SessionError::InWorktree(self.layout.project_dir.clone()))
            }
            Some(Checkout::Primary) => Ok(()),
        }
    }

    /// The branch `ws` has checked out; `HEAD` when it is detached (the
    /// rest of oxplow tolerates that, and the branch reconciler records
    /// a later checkout).
    async fn branch_of(&self, ws: &Path) -> String {
        self.vcs
            .head(ws)
            .await
            .ok()
            .and_then(|h| h.branch)
            .unwrap_or_else(|| "HEAD".to_string())
    }

    /// Idempotent: ensures a primary stream exists for this project.
    /// Reuses the existing one if present.
    pub async fn ensure_primary(&self) -> Result<Stream, SessionError> {
        self.validate_workspace().await?;
        if let Some(existing) = self.streams.primary().await? {
            return Ok(existing);
        }
        let branch = self.branch_of(&self.layout.project_dir).await;
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
            host: oxplow_domain::HostId::LOCAL,
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
        self.validate_workspace().await?;
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

        self.vcs
            .create_workspace(
                &self.layout.project_dir,
                &worktree_path,
                &branch,
                &branch_source,
            )
            .await?;

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
            host: oxplow_domain::HostId::LOCAL,
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
        self.validate_workspace().await?;
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
        let branch = self.branch_of(&worktree_path).await;

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
            host: oxplow_domain::HostId::LOCAL,
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

    /// Archive a stream and every thread under it. Soft-delete via
    /// `archived_at` — the rows stay in the DB so closed efforts,
    /// snapshots, and page_visit attribution don't dangle, but the
    /// rail and thread queries filter them out. If `delete_worktree`
    /// is true and the stream is a worktree (not primary), the
    /// on-disk working copy is removed too (`Vcs::remove_workspace`).
    /// Primary streams cannot be archived.
    /// Stream `id`, when it may be archived: it exists and isn't the
    /// primary. Asked before archiving changes anything.
    pub async fn archivable(&self, id: &StreamId) -> Result<Stream, SessionError> {
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
        Ok(stream)
    }

    pub async fn archive_stream(
        &self,
        id: &StreamId,
        delete_worktree: bool,
    ) -> Result<(), SessionError> {
        let stream = self.archivable(id).await?;
        // Archive every thread first so the rail+work surfaces drop
        // them in lockstep with the stream.
        let threads = self.threads.list_for_stream(id).await?;
        for t in threads {
            self.threads.archive(&t.id).await?;
        }
        self.streams.archive(id).await?;
        // Optional on-disk teardown. Best-effort: if the VCS refuses
        // (locked, …) the row is already archived, so the rail no longer
        // shows it; the user can clean up the directory manually.
        if delete_worktree {
            self.remove_workspace(&stream.worktree_path).await;
        }
        Ok(())
    }

    async fn remove_workspace(&self, path: &str) {
        if let Err(e) = self
            .vcs
            .remove_workspace(&self.layout.project_dir, Path::new(path))
            .await
        {
            tracing::warn!(path, error = %e, "couldn't remove the stream's working copy");
        }
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
        // Tear down the on-disk working copy first (best-effort — if it
        // fails, the user can still see the row and clean up manually).
        self.remove_workspace(&stream.worktree_path).await;
        self.streams.delete(id).await?;
        Ok(())
    }
}
