//! Cores for the `wiki` command module — file-backed knowledge base.

use oxplow_app::Services;
use oxplow_db::{WikiPage, WikiPageSearchHit};

use crate::error::IpcError;

pub async fn list_wiki_pages(svc: &Services) -> Result<Vec<WikiPage>, IpcError> {
    Ok(svc.wiki_page_store.list().await?)
}

pub async fn get_wiki_page(svc: &Services, slug: String) -> Result<Option<WikiPage>, IpcError> {
    Ok(svc.wiki_page_store.get(&slug).await?)
}

pub async fn upsert_wiki_page(svc: &Services, note: WikiPage) -> Result<(), IpcError> {
    Ok(svc.wiki_page_store.upsert(&note).await?)
}

pub async fn delete_wiki_page(svc: &Services, slug: String) -> Result<(), IpcError> {
    Ok(svc.wiki_page_store.delete(&slug).await?)
}

pub async fn search_wiki_titles(
    svc: &Services,
    query: String,
    limit: u32,
) -> Result<Vec<WikiPage>, IpcError> {
    Ok(svc
        .wiki_page_store
        .search_titles(&query, limit as usize)
        .await?)
}

pub async fn search_wiki_bodies(
    svc: &Services,
    query: String,
    limit: u32,
) -> Result<Vec<WikiPageSearchHit>, IpcError> {
    Ok(svc
        .wiki_page_store
        .search_bodies(&query, limit as usize)
        .await?)
}

fn wiki_page_body_path(svc: &Services, slug: &str) -> std::path::PathBuf {
    svc.layout
        .project_dir
        .join(".oxplow")
        .join("wiki")
        .join(format!("{slug}.md"))
}

pub async fn read_wiki_page_body(svc: &Services, slug: String) -> Result<String, IpcError> {
    let path = wiki_page_body_path(svc, &slug);
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&path).unwrap_or_default())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn write_wiki_page_body(
    svc: &Services,
    slug: String,
    body: String,
) -> Result<(), IpcError> {
    let path = wiki_page_body_path(svc, &slug);
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_wiki_pages_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_wiki_pages", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn read_wiki_page_body_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "read_wiki_page_body",
            serde_json::json!({ "slug": "no-such-page" }),
            &svc,
        )
        .await
        .unwrap();
        // Missing files read as empty string, not an error.
        assert_eq!(out, serde_json::json!(""));
    }
}
