//! Wiki pages — file-backed knowledge base.

use oxplow_domain::{ProseAudience, ProseVariants};

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

/// On-disk filename for an audience variant of a wiki body. Developer
/// is the canonical `<slug>.md`; executive/caveman are sibling files
/// `<slug>.executive.md` / `<slug>.caveman.md` (see
/// `oxplow_app::wiki_pages::wiki_slug_and_variant`, which routes those
/// siblings back to the base slug in the fs-watcher).
fn wiki_page_variant_path(
    state: &tauri::State<'_, AppState>,
    slug: &str,
    audience: ProseAudience,
) -> std::path::PathBuf {
    let name = match audience {
        ProseAudience::Developer => format!("{slug}.md"),
        ProseAudience::Executive => format!("{slug}.executive.md"),
        ProseAudience::Caveman => format!("{slug}.caveman.md"),
    };
    state
        .layout
        .project_dir
        .join(".oxplow")
        .join("wiki")
        .join(name)
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

/// Read one audience variant of a wiki body. Returns `""` when the
/// sibling file doesn't exist — callers fall back to the developer body.
#[tauri::command]
#[specta::specta]
pub async fn read_wiki_page_body_variant(
    state: tauri::State<'_, AppState>,
    slug: String,
    audience: ProseAudience,
) -> Result<String, IpcError> {
    let path = wiki_page_variant_path(&state, &slug, audience);
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&path).unwrap_or_default())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

/// Write one audience variant of a wiki body to its sibling file.
#[tauri::command]
#[specta::specta]
pub async fn write_wiki_page_body_variant(
    state: tauri::State<'_, AppState>,
    slug: String,
    audience: ProseAudience,
    body: String,
) -> Result<(), IpcError> {
    let path = wiki_page_variant_path(&state, &slug, audience);
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

/// Read all three audience variants of a wiki body at once. Developer
/// is the canonical body (empty string when the page has none yet);
/// executive/caveman are `None` when their sibling file is absent or
/// empty, so the frontend falls back to developer.
#[tauri::command]
#[specta::specta]
pub async fn list_wiki_page_variants(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<ProseVariants, IpcError> {
    let developer_path = wiki_page_variant_path(&state, &slug, ProseAudience::Developer);
    let executive_path = wiki_page_variant_path(&state, &slug, ProseAudience::Executive);
    let caveman_path = wiki_page_variant_path(&state, &slug, ProseAudience::Caveman);
    tokio::task::spawn_blocking(move || {
        let read_opt =
            |p: &std::path::Path| std::fs::read_to_string(p).ok().filter(|s| !s.is_empty());
        ProseVariants {
            developer: std::fs::read_to_string(&developer_path).unwrap_or_default(),
            executive: read_opt(&executive_path),
            caveman: read_opt(&caveman_path),
        }
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}
