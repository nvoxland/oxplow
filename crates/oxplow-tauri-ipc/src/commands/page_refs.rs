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

pub use oxplow_rpc::commands::page_refs::BacklinkEdge;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_backlinks(
    state: tauri::State<'_, AppState>,
    target_kind: String,
    target_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    oxplow_rpc::commands::page_refs::list_backlinks(&state, target_kind, target_id, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_outbound(
    state: tauri::State<'_, AppState>,
    source_kind: String,
    source_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    oxplow_rpc::commands::page_refs::list_outbound(&state, source_kind, source_id, limit).await
}
