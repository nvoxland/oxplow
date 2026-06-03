//! Wiki pages — file-backed knowledge base.

use oxplow_db::{WikiPage, WikiPageSearchHit};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_wiki_pages(state: tauri::State<'_, AppState>) -> Result<Vec<WikiPage>, IpcError> {
    Ok(state.wiki_page_store.list().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_wiki_page(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<Option<WikiPage>, IpcError> {
    Ok(state.wiki_page_store.get(&slug).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_wiki_page(
    state: tauri::State<'_, AppState>,
    note: WikiPage,
) -> Result<(), IpcError> {
    Ok(state.wiki_page_store.upsert(&note).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_wiki_page(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<(), IpcError> {
    Ok(state.wiki_page_store.delete(&slug).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn search_wiki_titles(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<WikiPage>, IpcError> {
    Ok(state
        .wiki_page_store
        .search_titles(&query, limit as usize)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn search_wiki_bodies(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<WikiPageSearchHit>, IpcError> {
    Ok(state
        .wiki_page_store
        .search_bodies(&query, limit as usize)
        .await?)
}

fn wiki_page_body_path(state: &tauri::State<'_, AppState>, slug: &str) -> std::path::PathBuf {
    state
        .layout
        .project_dir
        .join(".oxplow")
        .join("wiki")
        .join(format!("{slug}.md"))
}

#[tauri::command]
#[specta::specta]
pub async fn read_wiki_page_body(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<String, IpcError> {
    let path = wiki_page_body_path(&state, &slug);
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&path).unwrap_or_default())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn write_wiki_page_body(
    state: tauri::State<'_, AppState>,
    slug: String,
    body: String,
) -> Result<(), IpcError> {
    let path = wiki_page_body_path(&state, &slug);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}
