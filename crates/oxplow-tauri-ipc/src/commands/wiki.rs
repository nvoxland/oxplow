//! Wiki pages — file-backed knowledge base.

use oxplow_db::WikiPage;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_wiki_pages(state: tauri::State<'_, AppState>) -> Result<Vec<WikiPage>, IpcError> {
    oxplow_rpc::commands::wiki::list_wiki_pages(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_wiki_page(
    state: tauri::State<'_, AppState>,
    note: WikiPage,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::wiki::upsert_wiki_page(&state, note).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_wiki_page(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::wiki::delete_wiki_page(&state, slug).await
}

#[tauri::command]
#[specta::specta]
pub async fn search_wiki_titles(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<WikiPage>, IpcError> {
    oxplow_rpc::commands::wiki::search_wiki_titles(&state, query, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn read_wiki_page_body(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<String, IpcError> {
    oxplow_rpc::commands::wiki::read_wiki_page_body(&state, slug).await
}

#[tauri::command]
#[specta::specta]
pub async fn write_wiki_page_body(
    state: tauri::State<'_, AppState>,
    slug: String,
    body: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::wiki::write_wiki_page_body(&state, slug, body).await
}
