//! Unified cross-page reference graph reader.
//!
//! Both directions of the edge are exposed:
//! - `list_backlinks(target_kind, target_id)` — what points AT this
//!   page. Drives the Backlinks dropdown / panel for every page kind.
//! - `list_outbound(source_kind, source_id)` — what this page points
//!   to. Drives the new Outbound dropdown.
//!
//! The reader joins source labels (wiki title, task title,
//! commit subject) at read time so the renderer doesn't need to do
//! a second round-trip per row. Labels are best-effort — when the
//! source is gone (e.g. a deleted task) the label is `None`
//! and the renderer falls back to `source_id`.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_db::PageRefEdge;

use crate::error::IpcError;
use crate::state::AppState;

/// Edge plus a best-effort renderer label for the source.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklinkEdge {
    pub source_kind: String,
    pub source_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub ref_type: String,
    pub source_extra: Option<String>,
    /// Human label for the source (wiki title, task title,
    /// commit subject, …). Falls back to `source_id` in the
    /// renderer when None.
    pub source_label: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn list_backlinks(
    state: tauri::State<'_, AppState>,
    target_kind: String,
    target_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = state
        .page_ref_store
        .list_backlinks(&target_kind, &target_id, limit)
        .await?;
    Ok(decorate_with_labels(&state, edges).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_outbound(
    state: tauri::State<'_, AppState>,
    source_kind: String,
    source_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = state
        .page_ref_store
        .list_outbound(&source_kind, &source_id, limit)
        .await?;
    // For outbound, the "label" we want is for the *target*. We
    // keep the same struct shape, but populate `source_label` with
    // the target's label so the renderer can be kind-agnostic. When
    // the target kind has no first-class label (files, directories),
    // leave `source_label` as None — the frontend falls back to
    // `target_id`, which IS the meaningful display for those kinds.
    // Folding in the source's label here would stamp the current
    // page's own title on every file/dir row.
    Ok(decorate_outbound_targets(&state, edges).await)
}

async fn decorate_with_labels(state: &AppState, edges: Vec<PageRefEdge>) -> Vec<BacklinkEdge> {
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let label = source_label(state, &e.source_kind, &e.source_id).await;
        out.push(BacklinkEdge {
            source_kind: e.source_kind,
            source_id: e.source_id,
            target_kind: e.target_kind,
            target_id: e.target_id,
            ref_type: e.ref_type,
            source_extra: e.source_extra,
            source_label: label,
        });
    }
    out
}

async fn decorate_outbound_targets(state: &AppState, edges: Vec<PageRefEdge>) -> Vec<BacklinkEdge> {
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let label = source_label(state, &e.target_kind, &e.target_id).await;
        out.push(BacklinkEdge {
            source_kind: e.source_kind,
            source_id: e.source_id,
            target_kind: e.target_kind,
            target_id: e.target_id,
            ref_type: e.ref_type,
            source_extra: e.source_extra,
            source_label: label,
        });
    }
    out
}

/// Best-effort label lookup by kind. Delegates to the shared typed
/// [`ref_resolver`] (the single source of truth for hydrating a
/// `(kind,id)` ref) and keeps only its `title`. Returns `None` when the
/// row doesn't exist (deleted) or the kind carries no first-class label
/// (files/directories use the path itself).
async fn source_label(state: &AppState, kind: &str, id: &str) -> Option<String> {
    oxplow_app::ref_resolver::resolve_ref(state, kind, id)
        .await
        .title
}
