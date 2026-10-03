//! Cores for the `comments` command module — the reads of the threaded
//! annotations anchored to a text selection on any page. Writes are the
//! `knowledge.*` comment commands (the renderer's anchor re-locate
//! included); views re-read `v_comment`.

use oxplow_app::Services;
use oxplow_domain::stores::CommentStore;
use oxplow_domain::{CommentTarget, CommentThread, StreamId};

use crate::error::IpcError;

pub async fn list_comments_for_target(
    svc: &Services,
    target_kind: String,
    target_id: String,
) -> Result<Vec<CommentThread>, IpcError> {
    let target = CommentTarget {
        kind: target_kind,
        id: target_id,
    };
    Ok(svc.comment_store.list_for_target(&target).await?)
}

pub async fn list_comments_for_stream(
    svc: &Services,
    stream_id: StreamId,
) -> Result<Vec<CommentThread>, IpcError> {
    Ok(svc.comment_store.list_for_stream(&stream_id).await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_comments_for_target_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_comments_for_target",
            serde_json::json!({"targetKind": "wiki", "targetId": "some-slug"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_comments_for_stream_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_comments_for_stream",
            serde_json::json!({"streamId": "str1"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
