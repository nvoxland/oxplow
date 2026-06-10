//! Cores for the `search` command module — site-wide BM25 search.

use oxplow_app::Services;
use oxplow_db::SearchHit;

use crate::error::IpcError;

/// Site-wide BM25 search across tasks, comments, notes, wiki pages, and
/// per-stream file contents. `stream_id` scopes file/stream-bound hits to one
/// worktree (project-global hits like wiki always included); `None` searches
/// everything. `kinds` optionally restricts to a subset
/// (`task|comment|note|wiki|file`). Results are ranked best-first.
pub async fn search(
    svc: &Services,
    query: String,
    stream_id: Option<String>,
    kinds: Option<Vec<String>>,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, IpcError> {
    let kinds = kinds.unwrap_or_default();
    Ok(svc
        .search_store
        .search(
            &query,
            stream_id.as_deref(),
            &kinds,
            limit.unwrap_or(50) as usize,
        )
        .await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn search_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("search", serde_json::json!({ "query": "anything" }), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }
}
