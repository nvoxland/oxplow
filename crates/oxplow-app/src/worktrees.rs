//! Stream → workspace routing (`.context/vcs.md`). A stream works in one
//! directory — the primary checkout or its own isolated workspace — and
//! everything that takes a stream (file I/O, VCS reads, commands) asks
//! here where that is. The VCS provider itself only ever sees paths.
//!
//! The stream store is the truth (`stream.worktree_path`, which never
//! changes for a stream); the router memoizes each lookup, and a deleted
//! stream is forgotten.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{DomainError, Stream, StreamId};
use tokio::sync::RwLock;

/// Where a stream whose `worktree_path` is `worktree_path` works: the path
/// itself when absolute, else under the primary checkout.
pub fn workspace_path(project_dir: &Path, worktree_path: &str) -> PathBuf {
    let raw = PathBuf::from(worktree_path);
    if raw.is_absolute() {
        raw
    } else {
        project_dir.join(raw)
    }
}

pub struct WorktreeRouter {
    project_dir: PathBuf,
    streams: Arc<dyn StreamStore>,
    memo: RwLock<HashMap<StreamId, PathBuf>>,
}

impl WorktreeRouter {
    pub fn new(project_dir: PathBuf, streams: Arc<dyn StreamStore>) -> Self {
        Self {
            project_dir,
            streams,
            memo: RwLock::new(HashMap::new()),
        }
    }

    /// The primary checkout.
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Where `stream_id` works; the primary checkout for `None`, a
    /// malformed id, or a stream that doesn't exist. For reads and for
    /// what genuinely defaults to the primary — a write names its stream
    /// through [`Self::resolve_strict`].
    pub async fn resolve(&self, stream_id: Option<&str>) -> PathBuf {
        match stream_id.and_then(StreamId::try_from_str) {
            Some(id) => self
                .lookup(&id)
                .await
                .unwrap_or_else(|| self.project_dir.clone()),
            None => self.project_dir.clone(),
        }
    }

    /// Where `stream_id` works, refusing rather than falling back to the
    /// primary checkout: a stream-scoped write that arrived without a
    /// stream that resolves (a field that didn't bind) must not land on
    /// the primary's branch.
    pub async fn resolve_strict(&self, stream_id: Option<&str>) -> Result<PathBuf, DomainError> {
        let Some(raw) = stream_id else {
            return Err(DomainError::Invalid(
                "this operation needs a stream; refusing to fall back to the primary worktree"
                    .into(),
            ));
        };
        let id = StreamId::try_from_str(raw)
            .ok_or_else(|| DomainError::Invalid(format!("{raw:?} is not a valid stream id")))?;
        self.lookup(&id)
            .await
            .ok_or_else(|| DomainError::Invalid(format!("no stream {raw:?}")))
    }

    async fn lookup(&self, id: &StreamId) -> Option<PathBuf> {
        if let Some(p) = self.memo.read().await.get(id) {
            return Some(p.clone());
        }
        let stream = self.streams.get(id).await.ok()??;
        let path = self.path_of(&stream);
        self.memo.write().await.insert(*id, path.clone());
        Some(path)
    }

    fn path_of(&self, stream: &Stream) -> PathBuf {
        workspace_path(&self.project_dir, &stream.worktree_path)
    }

    /// Forget a deleted stream.
    pub async fn forget(&self, stream_id: &StreamId) {
        self.memo.write().await.remove(stream_id);
    }

    /// Every stream with its worktree.
    pub async fn all(&self) -> Result<Vec<(StreamId, PathBuf)>, DomainError> {
        Ok(self
            .streams
            .list()
            .await?
            .iter()
            .map(|s| (s.id, self.path_of(s)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P5.B1 (tsk520): a stream routes to its worktree; an unknown or
    /// absent stream reads the primary checkout but a strict resolve
    /// refuses it.
    #[tokio::test]
    async fn streams_route_to_their_worktrees_and_strict_refuses_the_rest() {
        let f = crate::test_fixtures::services_with_effort().await;
        let router = &f.svc.worktrees;
        let primary = router.project_dir().to_path_buf();
        let stream = f.svc.stream_store.list().await.unwrap().remove(0);
        let id = stream.id.to_string();
        assert_eq!(router.resolve(Some(&id)).await, router.path_of(&stream));
        assert_eq!(router.resolve(None).await, primary);
        assert_eq!(router.resolve(Some("str999999")).await, primary);
        assert!(router.resolve_strict(None).await.is_err());
        assert!(router.resolve_strict(Some("str999999")).await.is_err());
        assert!(router.resolve_strict(Some("nonsense")).await.is_err());
        assert_eq!(
            router.resolve_strict(Some(&id)).await.unwrap(),
            router.path_of(&stream)
        );
    }
}
