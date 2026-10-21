//! Cores for the `search` command module — site-wide BM25 search.

use oxplow_app::Services;
use oxplow_db::SearchHit;

use crate::error::IpcError;

/// Site-wide BM25 search across every searchable resource
/// (`v_resource.searchable`) and per-stream file contents; each hit names
/// the URI it found. `stream_id` scopes file/stream-bound hits to one
/// worktree (project-global hits like wiki always included); `None`
/// searches everything. `resources` optionally restricts to some of those
/// (`oxplow/wiki`, or `oxplow/file`). Results are ranked best-first.
pub async fn search(
    svc: &Services,
    query: String,
    stream_id: Option<String>,
    resources: Option<Vec<String>>,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, IpcError> {
    let resources = resources.unwrap_or_default();
    Ok(svc
        .search_store
        .search(
            &query,
            stream_id.as_deref(),
            &resources,
            limit.unwrap_or(50) as usize,
        )
        .await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn search_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch("search", serde_json::json!({ "query": "anything" }), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }
}
