//! Cores for the `page_visit` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::{OxplowEvent, Services};
use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VisitedPage {
    pub page_kind: String,
    pub page_id: String,
    pub visit_count: i64,
}

pub async fn record_page_visit(
    svc: &Services,
    page_kind: String,
    page_id: String,
    label: Option<String>,
    duration_ms: Option<i64>,
    thread_id: Option<String>,
) -> Result<PageVisit, IpcError> {
    let visit = svc
        .page_visit_store
        .record(
            &page_kind,
            &page_id,
            label.as_deref(),
            duration_ms,
            thread_id.as_deref(),
        )
        .await?;
    svc.events.emit(OxplowEvent::PageVisitChanged);
    Ok(visit)
}

pub async fn list_recent_page_visits(
    svc: &Services,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(svc
        .page_visit_store
        .list_recent(limit as usize, thread_id.as_deref())
        .await?)
}

pub async fn top_visited_pages(
    svc: &Services,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<VisitedPage>, IpcError> {
    let pairs = svc
        .page_visit_store
        .list_top(limit as usize, thread_id.as_deref())
        .await?;
    Ok(pairs
        .into_iter()
        .map(|(page_kind, page_id, visit_count)| VisitedPage {
            page_kind,
            page_id,
            visit_count,
        })
        .collect())
}

pub async fn forget_page(
    svc: &Services,
    page_kind: String,
    page_id: String,
) -> Result<(), IpcError> {
    svc.page_visit_store
        .forget_page(&page_kind, &page_id)
        .await?;
    svc.events.emit(OxplowEvent::PageVisitChanged);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_recent_page_visits_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_recent_page_visits",
            serde_json::json!({"limit": 10, "threadId": null}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
