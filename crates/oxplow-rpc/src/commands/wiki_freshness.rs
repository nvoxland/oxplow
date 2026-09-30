//! Cores for the `wiki_freshness` command module — wiki page
//! freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file ref the wiki page
//! carries: `KnowledgeProvider::freshness`, which reads `v_knowledge_ref`
//! — the one definition of staleness. Marking a ref verified is
//! `knowledge.write_page` with it in `verified_refs`.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::Services;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WikiRefFreshness {
    pub path: String,
    /// The snapshot the ref was pinned to (written or verified against);
    /// `None` if never pinned.
    pub pinned_snapshot_id: Option<i64>,
    /// The VCS revision nearest the pin, and whether the pinned snapshot
    /// is exactly it.
    pub pinned_vcs_rev: Option<String>,
    pub pinned_vcs_rev_exact: bool,
    /// The file's latest primary-stream snapshot; `None` if never
    /// captured there.
    pub latest_snapshot_id: Option<i64>,
    /// The file changed after the pin (or was captured but never
    /// pinned). The renderer paints a "stale" chip on these rows.
    pub stale: bool,
}

pub async fn list_wiki_freshness(
    svc: &Services,
    slug: String,
) -> Result<Vec<WikiRefFreshness>, IpcError> {
    let page = oxplow_app::knowledge::page_ref(&slug);
    let rows = svc
        .knowledge
        .freshness(&page)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|r| WikiRefFreshness {
            path: r
                .target
                .strip_prefix("file:")
                .unwrap_or(&r.target)
                .to_string(),
            pinned_snapshot_id: r.pinned_snapshot,
            pinned_vcs_rev: r.pinned_revision,
            pinned_vcs_rev_exact: r.pinned_revision_exact,
            latest_snapshot_id: r.latest_snapshot,
            stale: r.stale,
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
