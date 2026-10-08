//! The VCS reads a stream's UI and agent tools make, over the `Vcs`
//! capability (`.context/vcs.md`): each takes a stream (routed by
//! `WorktreeRouter`) and names versions as `Revision`s, so no caller
//! sees a provider's own ids or kinds.

use oxplow_domain::vcs::{
    BlameLine, Branch, Divergence, LogQuery, Revision, RevisionDetail, RevisionInfo, VcsWorkspace,
    WorkspaceStatus,
};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Services;

/// Where a stream's workspace is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HeadInfo {
    /// The head revision; `None` before the first commit.
    #[specta(type = Option<String>)]
    pub revision: Option<Revision>,
    /// The branch checked out; `None` when detached.
    pub branch: Option<String>,
}

fn vcs_revision(svc: &Services, rev: String) -> Revision {
    Revision::Vcs {
        kind: svc.vcs.rev_kind().into(),
        rev,
    }
}

/// The provider's own id for `rev`: `None` for the working tree; a
/// snapshot answers with the revision its tree equals, or is refused.
async fn provider_rev(svc: &Services, rev: &Revision) -> Result<Option<String>, DomainError> {
    match rev {
        Revision::Working => Ok(None),
        Revision::Vcs { kind, rev } if kind == svc.vcs.rev_kind() => Ok(Some(rev.clone())),
        Revision::Vcs { kind, .. } => Err(DomainError::Invalid(format!(
            "`{rev}` is a {kind} revision; this workspace is under {}",
            svc.vcs.rev_kind()
        ))),
        Revision::Snapshot(id) => match svc.trees.revision_of(*id).await? {
            Some(Revision::Vcs { rev, .. }) => Ok(Some(rev)),
            _ => Err(DomainError::Invalid(format!(
                "snapshot {id} wasn't taken at a revision"
            ))),
        },
    }
}

pub async fn head(svc: &Services, stream_id: Option<&str>) -> Result<HeadInfo, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let head = svc.vcs.head(&ws).await?;
    Ok(HeadInfo {
        revision: head.revision.map(|r| vcs_revision(svc, r)),
        branch: head.branch,
    })
}

pub async fn status(
    svc: &Services,
    stream_id: Option<&str>,
) -> Result<WorkspaceStatus, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    Ok(svc.vcs.status(&ws).await?)
}

/// Who last changed each line of `path` at `revision` (the working tree:
/// uncommitted lines name no revision).
pub async fn blame(
    svc: &Services,
    stream_id: Option<&str>,
    path: &str,
    revision: &Revision,
) -> Result<Vec<BlameLine>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let rev = provider_rev(svc, revision).await?;
    Ok(svc.vcs.blame(&ws, path, rev.as_deref()).await?)
}

/// A VCS revision's message and changed files; `None` for one the
/// workspace doesn't have.
pub async fn revision(
    svc: &Services,
    stream_id: Option<&str>,
    revision: &Revision,
) -> Result<Option<RevisionDetail>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let Some(rev) = provider_rev(svc, revision).await? else {
        return Err(DomainError::Invalid(
            "the working tree isn't a revision".into(),
        ));
    };
    Ok(svc.vcs.revision(&ws, &rev).await?)
}

/// Where the histories of `a` and `b` fork.
pub async fn merge_base(
    svc: &Services,
    stream_id: Option<&str>,
    a: &Revision,
    b: &Revision,
) -> Result<Option<Revision>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let (Some(a), Some(b)) = (provider_rev(svc, a).await?, provider_rev(svc, b).await?) else {
        return Err(DomainError::Invalid(
            "a merge base is between two revisions".into(),
        ));
    };
    Ok(svc
        .vcs
        .merge_base(&ws, &a, &b)
        .await?
        .map(|r| vcs_revision(svc, r)))
}

/// The provider's ids for two revisions that must both be VCS revisions.
async fn provider_pair(
    svc: &Services,
    a: &Revision,
    b: &Revision,
) -> Result<(String, String), DomainError> {
    match (provider_rev(svc, a).await?, provider_rev(svc, b).await?) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(DomainError::Invalid(
            "this compares two revisions, not the working tree".into(),
        )),
    }
}

/// The stream's history from its head, newest first — or every branch's
/// with `all`. (The UI reads history from `v_commit`; this is the live
/// walk for agents.)
pub async fn log(
    svc: &Services,
    stream_id: Option<&str>,
    limit: Option<u32>,
    all: bool,
) -> Result<Vec<RevisionInfo>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    Ok(svc.vcs.log(&ws, LogQuery { limit, all }).await?)
}

pub async fn branches(svc: &Services, stream_id: Option<&str>) -> Result<Vec<Branch>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    Ok(svc.vcs.branches(&ws).await?)
}

/// How far `head` and `base` have diverged, and whether `head` would
/// merge cleanly.
pub async fn divergence(
    svc: &Services,
    stream_id: Option<&str>,
    base: &Revision,
    head: &Revision,
) -> Result<Divergence, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let (base, head) = provider_pair(svc, base, head).await?;
    Ok(svc.vcs.divergence(&ws, &base, &head).await?)
}

/// Revisions on `head` that `base` lacks, newest first.
pub async fn revisions_between(
    svc: &Services,
    stream_id: Option<&str>,
    base: &Revision,
    head: &Revision,
    limit: u32,
) -> Result<Vec<RevisionInfo>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    let (base, head) = provider_pair(svc, base, head).await?;
    Ok(svc.vcs.revisions_between(&ws, &base, &head, limit).await?)
}

pub async fn file_history(
    svc: &Services,
    stream_id: Option<&str>,
    path: &str,
    limit: u32,
) -> Result<Vec<RevisionInfo>, DomainError> {
    let ws = svc.worktrees.resolve(stream_id).await.into_local_path();
    Ok(svc.vcs.file_history(&ws, path, limit).await?)
}

/// Working copies of the repository no stream uses yet — what "adopt a
/// worktree" offers.
pub async fn adoptable_workspaces(svc: &Services) -> Result<Vec<VcsWorkspace>, DomainError> {
    use oxplow_domain::stores::StreamStore as _;
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    let registered: Vec<std::path::PathBuf> = svc
        .stream_store
        .list()
        .await?
        .iter()
        .map(|s| canon(&s.worktree_path))
        .collect();
    Ok(svc
        .vcs
        .list_workspaces(svc.worktrees.project_dir())
        .await?
        .into_iter()
        .filter(|w| !w.is_main && !registered.contains(&canon(&w.path)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{commit_all, services_with_effort};

    /// P5.B4 (tsk523): the stream reads name versions as revisions — the
    /// head, a commit's detail, blame of the working file, a merge base.
    #[tokio::test]
    async fn stream_reads_speak_revisions() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let ws = svc.layout.project_dir.clone();
        std::fs::write(ws.join("a.txt"), "one\n").unwrap();
        let sha = commit_all(&ws, "c1");
        let rev = Revision::git(sha.clone());
        let h = head(svc, None).await.unwrap();
        assert_eq!(h.revision, Some(rev.clone()));
        let detail = revision(svc, None, &Revision::git("HEAD"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.info.id, sha);
        assert!(revision(svc, None, &Revision::Working).await.is_err());
        std::fs::write(ws.join("a.txt"), "one\nnew\n").unwrap();
        let lines = blame(svc, None, "a.txt", &Revision::Working).await.unwrap();
        assert_eq!(
            lines.iter().map(|l| l.revision.clone()).collect::<Vec<_>>(),
            vec![Some(sha.clone()), None]
        );
        assert_eq!(
            merge_base(svc, None, &Revision::git("HEAD"), &rev)
                .await
                .unwrap(),
            Some(rev)
        );
        let st = status(svc, None).await.unwrap();
        assert!(st.entries.iter().any(|e| e.path == "a.txt"), "{st:?}");
    }
}
