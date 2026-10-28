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
    crate::commands::bus::read_as_person(
        svc,
        "oxplow.search.find",
        serde_json::json!({
            "query": query,
            "stream_id": stream_id,
            "resources": resources,
            "limit": limit,
        }),
    )
    .await
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

    /// The person's search is the agent's: a stream id that isn't one is
    /// refused at its field, on either front door.
    #[tokio::test]
    async fn a_thread_id_as_the_stream_is_refused() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch(
            "search",
            serde_json::json!({ "query": "anything", "streamId": "thr1" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID", "{err:?}");
        assert_eq!(err.field.as_deref(), Some("/stream_id"), "{err:?}");
    }
}
