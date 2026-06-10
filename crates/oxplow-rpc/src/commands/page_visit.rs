//! Cores for the `page_visit` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::{OxplowEvent, Services};
use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use oxplow_domain::stores::TaskStore as _;
use oxplow_domain::{TaskStatus, ThreadId, Timestamp};
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PageVisitDay {
    pub day: String,
    pub count: i64,
}

pub async fn list_frequent_usage(svc: &Services, limit: u32) -> Result<Vec<PageVisit>, IpcError> {
    Ok(svc.page_visit_store.list_frequent(limit as usize).await?)
}

/// Pages currently kept open in editor tabs (best-effort: derived from
/// recent visits whose duration_ms is null — i.e. the open-event hasn't
/// been closed yet). The renderer already filters to its own tab list.
pub async fn list_currently_open_usage(
    svc: &Services,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    let recent = svc
        .page_visit_store
        .list_recent(limit as usize * 4, None)
        .await?;
    Ok(recent
        .into_iter()
        .filter(|v| v.duration_ms.is_none())
        .take(limit as usize)
        .collect())
}

/// Recently completed tasks merged with recently updated wiki
/// notes, sorted by timestamp DESC. Drives the rail's "Finished"
/// section. Items whose timestamp is `<= finished_cleared_at` are
/// hidden until something newer lands.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FinishedEntry {
    #[serde(rename = "task")]
    Task {
        #[serde(rename = "itemId")]
        item_id: i64,
        title: String,
        t: Timestamp,
    },
    #[serde(rename = "wiki")]
    Wiki {
        slug: String,
        title: String,
        t: Timestamp,
    },
}

impl FinishedEntry {
    fn timestamp(&self) -> Timestamp {
        match self {
            FinishedEntry::Task { t, .. } => *t,
            FinishedEntry::Wiki { t, .. } => *t,
        }
    }
}

pub async fn list_recently_finished(
    svc: &Services,
    thread_id: Option<String>,
    limit: u32,
) -> Result<Vec<FinishedEntry>, IpcError> {
    let cap = limit.max(1) as usize;
    let cursor_key = thread_id.clone().unwrap_or_default();
    let cleared_at = svc
        .finished_cleared_at
        .read()
        .expect("finished_cleared_at rwlock")
        .get(&cursor_key)
        .copied();

    let mut entries: Vec<FinishedEntry> = Vec::new();

    if let Some(tid) = thread_id.as_ref() {
        // Thread-scoped: only items filed against this thread, only
        // wiki pages the thread actually touched.
        let tid = ThreadId::try_from_str(tid).unwrap_or(ThreadId::placeholder());
        let items = svc.task_store.list_for_thread(&tid).await?;
        for item in items {
            if item.status != TaskStatus::Done {
                continue;
            }
            let Some(t) = item.completed_at else { continue };
            entries.push(FinishedEntry::Task {
                item_id: item.id.value(),
                title: item.title,
                t,
            });
        }
        let touches = svc
            .wiki_page_thread_updates
            .list_for_thread(&tid, cap * 4)
            .await?;
        for touch in touches {
            let Some(page) = svc.wiki_page_store.get(&touch.slug).await? else {
                continue;
            };
            entries.push(FinishedEntry::Wiki {
                slug: page.slug,
                title: page.title,
                // Use the per-thread timestamp so a different thread
                // editing the same page doesn't promote this thread's
                // entry.
                t: touch.last_seen_at,
            });
        }
    } else {
        // No thread context — fall back to a global view (used for
        // initial paint before a thread is selected).
        let done = svc.task_store.list_recently_done(cap).await?;
        for item in done {
            let Some(t) = item.completed_at else { continue };
            entries.push(FinishedEntry::Task {
                item_id: item.id.value(),
                title: item.title,
                t,
            });
        }
        let pages = svc.wiki_page_store.list().await?;
        for page in pages.into_iter().take(cap) {
            entries.push(FinishedEntry::Wiki {
                slug: page.slug,
                title: page.title,
                t: page.updated_at,
            });
        }
    }

    entries.retain(|e| match cleared_at {
        Some(cursor) => e.timestamp() > cursor,
        None => true,
    });
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp()));
    entries.truncate(cap);
    Ok(entries)
}

/// Hide the current "Finished" entries behind a cursor. Source rows
/// (tasks / wiki pages) are untouched; new finishes still surface
/// because their timestamp is newer than the cursor. Cursor is
/// per-thread so clearing one thread's section doesn't blank another.
pub async fn clear_recently_finished(
    svc: &Services,
    thread_id: Option<String>,
) -> Result<(), IpcError> {
    let key = thread_id.unwrap_or_default();
    svc.finished_cleared_at
        .write()
        .expect("finished_cleared_at rwlock")
        .insert(key, Timestamp::now());
    svc.events.emit(OxplowEvent::PageVisitChanged);
    Ok(())
}

pub async fn count_page_visits_by_day(
    svc: &Services,
    days: u32,
) -> Result<Vec<PageVisitDay>, IpcError> {
    let rows = svc.page_visit_store.count_by_day(days).await?;
    Ok(rows
        .into_iter()
        .map(|(day, count)| PageVisitDay { day, count })
        .collect())
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

    #[tokio::test]
    async fn list_recently_finished_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_recently_finished",
            serde_json::json!({"threadId": "thr1", "limit": 5}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
