//! Tauri adapters for the `dashboards` command module (tsk138) — one-line
//! delegates to the `oxplow-rpc` cores. Request structs are re-exported so the
//! specta TS export sees them.

use oxplow_db::{Dashboard, DashboardWithItems};
use oxplow_domain::{DashboardId, DashboardItemId};

pub use oxplow_rpc::commands::dashboards::{
    AddDashboardItemRequest, RenameDashboardRequest, ReorderDashboardItemsRequest,
    UpdateDashboardItemRequest,
};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_dashboards(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Dashboard>, IpcError> {
    oxplow_rpc::commands::dashboards::list_dashboards(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_dashboard(
    state: tauri::State<'_, AppState>,
    id: DashboardId,
) -> Result<Option<DashboardWithItems>, IpcError> {
    oxplow_rpc::commands::dashboards::get_dashboard(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_dashboard(
    state: tauri::State<'_, AppState>,
    title: String,
) -> Result<Dashboard, IpcError> {
    oxplow_rpc::commands::dashboards::create_dashboard(&state, title).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_dashboard(
    state: tauri::State<'_, AppState>,
    req: RenameDashboardRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::dashboards::rename_dashboard(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_dashboard(
    state: tauri::State<'_, AppState>,
    id: DashboardId,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::dashboards::delete_dashboard(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn add_dashboard_item(
    state: tauri::State<'_, AppState>,
    req: AddDashboardItemRequest,
) -> Result<DashboardItemId, IpcError> {
    oxplow_rpc::commands::dashboards::add_dashboard_item(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_dashboard_item(
    state: tauri::State<'_, AppState>,
    req: UpdateDashboardItemRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::dashboards::update_dashboard_item(&state, req).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_dashboard_item(
    state: tauri::State<'_, AppState>,
    id: DashboardItemId,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::dashboards::remove_dashboard_item(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_dashboard_items(
    state: tauri::State<'_, AppState>,
    req: ReorderDashboardItemsRequest,
) -> Result<(), IpcError> {
    oxplow_rpc::commands::dashboards::reorder_dashboard_items(&state, req).await
}
