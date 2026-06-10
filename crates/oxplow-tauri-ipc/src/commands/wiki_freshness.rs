//! Wiki page freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file/directory
//! ref the wiki page carries, joining the captured snapshot pin on
//! `page_ref` with the latest `file_snapshot` for that path so the
//! UI can render a per-ref staleness flag. `mark_wiki_ref_verified`
//! and `mark_all_wiki_refs_verified` re-stamp the pin to the
//! current resolved version when the user explicitly confirms the
//! page is still accurate.

pub use oxplow_rpc::commands::wiki_freshness::WikiRefFreshness;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_wiki_freshness(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<Vec<WikiRefFreshness>, IpcError> {
    oxplow_rpc::commands::wiki_freshness::list_wiki_freshness(&state, slug).await
}

#[tauri::command]
#[specta::specta]
pub async fn mark_wiki_ref_verified(
    state: tauri::State<'_, AppState>,
    slug: String,
    path: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::wiki_freshness::mark_wiki_ref_verified(&state, slug, path).await
}

#[tauri::command]
#[specta::specta]
pub async fn mark_all_wiki_refs_verified(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<usize, IpcError> {
    oxplow_rpc::commands::wiki_freshness::mark_all_wiki_refs_verified(&state, slug).await
}
