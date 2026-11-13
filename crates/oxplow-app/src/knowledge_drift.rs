//! Drift detail for a knowledge page's file ref — the knowledge-named face of
//! [`crate::pin_drift`] (`oxplow_knowledge_drift`), kept until the knowledge
//! capability goes.

use std::path::Path;

use oxplow_db::{SqlitePageRefStore, SqliteSnapshotStore};
use oxplow_domain::DomainError;
use serde::Serialize;

pub use crate::pin_drift::unified_diff;

/// Outcome of a drift query for one `(knowledge page, file ref)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgeDrift {
    /// The page's URI.
    pub page: String,
    pub path: String,
    pub pinned_snapshot_id: Option<i64>,
    /// See [`crate::pin_drift::PinDrift::status`].
    pub status: String,
    pub unified_diff: Option<String>,
    pub truncated: bool,
}

/// Compute drift for one of a knowledge page's file refs.
pub async fn compute_knowledge_drift(
    page_refs: &SqlitePageRefStore,
    snapshots: &SqliteSnapshotStore,
    content: &crate::snapshot_content::SnapshotContent,
    project_dir: &Path,
    page: &str,
    path: &str,
) -> Result<KnowledgeDrift, DomainError> {
    let d =
        crate::pin_drift::compute_pin_drift(page_refs, snapshots, content, project_dir, page, path)
            .await?;
    Ok(KnowledgeDrift {
        page: d.record,
        path: d.path,
        pinned_snapshot_id: d.pinned_snapshot_id,
        status: d.status,
        unified_diff: d.unified_diff,
        truncated: d.truncated,
    })
}
