//! Cores for the `wiki_freshness` command module — wiki page
//! freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file/directory
//! ref the wiki page carries, joining the captured snapshot pin on
//! `page_ref` with the latest `file_snapshot` for that path so the
//! UI can render a per-ref staleness flag. Marking a ref verified is
//! `knowledge.write_page` with it in `verified_refs`.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::Services;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WikiRefFreshness {
    pub path: String,
    /// The snapshot the ref was captured against. 0 when the
    /// wiki sync had no snapshot service available.
    pub local_snapshot_id: i64,
    /// Closest known git commit at capture time; populated only
    /// when the worktree had a HEAD.
    pub closest_vcs_rev: Option<String>,
    /// `true` when the local snapshot is byte-equal to the recorded
    /// commit (capture was on a clean worktree, or
    /// `set_snapshot_git_commit` later attached HEAD to the snapshot).
    pub vcs_rev_exact: bool,
    /// The latest `snapshot.id` whose `file_snapshot.path` matches
    /// this target. `None` when the file hasn't been captured (e.g.
    /// it's outside the workspace or has never been touched since
    /// the snapshot service booted).
    pub latest_snapshot_id: Option<i64>,
    /// `true` when `latest_snapshot_id > local_snapshot_id`. The
    /// renderer paints a "stale" chip on these rows.
    pub stale: bool,
}

pub async fn list_wiki_freshness(
    svc: &Services,
    slug: String,
) -> Result<Vec<WikiRefFreshness>, IpcError> {
    let raw = svc.page_ref_store.list_wiki_file_freshness(&slug).await?;
    Ok(raw
        .into_iter()
        .map(|(path, local, git, exact, latest)| WikiRefFreshness {
            path,
            local_snapshot_id: local.unwrap_or(0),
            closest_vcs_rev: git,
            vcs_rev_exact: exact,
            latest_snapshot_id: latest,
            stale: matches!((latest, local), (Some(l), Some(loc)) if l > loc)
                || matches!((latest, local), (Some(_), None)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_wiki_freshness_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_wiki_freshness",
            serde_json::json!({ "slug": "no-such-page" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
