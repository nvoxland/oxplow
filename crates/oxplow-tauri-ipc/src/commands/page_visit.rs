use oxplow_db::PageVisit;

use crate::error::IpcError;
use crate::state::AppState;

pub use oxplow_rpc::commands::page_visit::{FinishedEntry, PageVisitDay, VisitedPage};

#[tauri::command]
#[specta::specta]
pub async fn record_page_visit(
    state: tauri::State<'_, AppState>,
    page_kind: String,
    page_id: String,
    label: Option<String>,
    duration_ms: Option<i64>,
    thread_id: Option<String>,
) -> Result<PageVisit, IpcError> {
    oxplow_rpc::commands::page_visit::record_page_visit(
        &state,
        page_kind,
        page_id,
        label,
        duration_ms,
        thread_id,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_page_visits(
    state: tauri::State<'_, AppState>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<PageVisit>, IpcError> {
    oxplow_rpc::commands::page_visit::list_recent_page_visits(&state, limit, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn top_visited_pages(
    state: tauri::State<'_, AppState>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<VisitedPage>, IpcError> {
    oxplow_rpc::commands::page_visit::top_visited_pages(&state, limit, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn forget_page(
    state: tauri::State<'_, AppState>,
    page_kind: String,
    page_id: String,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::page_visit::forget_page(&state, page_kind, page_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_frequent_usage(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    oxplow_rpc::commands::page_visit::list_frequent_usage(&state, limit).await
}

/// Pages currently kept open in editor tabs (best-effort: derived from
/// recent visits whose duration_ms is null — i.e. the open-event hasn't
/// been closed yet). The renderer already filters to its own tab list.
#[tauri::command]
#[specta::specta]
pub async fn list_currently_open_usage(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    oxplow_rpc::commands::page_visit::list_currently_open_usage(&state, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_recently_finished(
    state: tauri::State<'_, AppState>,
    thread_id: Option<String>,
    limit: u32,
) -> Result<Vec<FinishedEntry>, IpcError> {
    oxplow_rpc::commands::page_visit::list_recently_finished(&state, thread_id, limit).await
}

/// Hide the current "Finished" entries behind a cursor. Source rows
/// (tasks / wiki pages) are untouched; new finishes still surface
/// because their timestamp is newer than the cursor. Cursor is
/// per-thread so clearing one thread's section doesn't blank another.
#[tauri::command]
#[specta::specta]
pub async fn clear_recently_finished(
    state: tauri::State<'_, AppState>,
    thread_id: Option<String>,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::page_visit::clear_recently_finished(&state, thread_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn count_page_visits_by_day(
    state: tauri::State<'_, AppState>,
    days: u32,
) -> Result<Vec<PageVisitDay>, IpcError> {
    oxplow_rpc::commands::page_visit::count_page_visits_by_day(&state, days).await
}
