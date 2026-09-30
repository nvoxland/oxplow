//! Cores for the `dashboards` command module (epic tsk138) — user-created
//! dashboards of metric tiles. Project-global; every write emits
//! `OxplowEvent::DashboardsChanged` so agent- and UI-driven edits both
//! live-refresh the renderer.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::{OxplowEvent, Services};
use oxplow_db::{Dashboard, DashboardWithItems};
use oxplow_domain::{DashboardId, DashboardItemId};

use crate::error::IpcError;

pub async fn list_dashboards(svc: &Services) -> Result<Vec<Dashboard>, IpcError> {
    Ok(svc.dashboard_store.list().await?)
}

pub async fn get_dashboard(
    svc: &Services,
    id: DashboardId,
) -> Result<Option<DashboardWithItems>, IpcError> {
    Ok(svc.dashboard_store.get(id).await?)
}

/// Create an empty dashboard and return it (for the create-then-navigate flow).
pub async fn create_dashboard(svc: &Services, title: String) -> Result<Dashboard, IpcError> {
    let id = svc.dashboard_store.create(title).await?;
    let created = svc
        .dashboard_store
        .get(id)
        .await?
        .ok_or_else(|| IpcError::internal("created dashboard vanished"))?
        .dashboard;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(created)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameDashboardRequest {
    pub id: DashboardId,
    pub title: String,
}

pub async fn rename_dashboard(svc: &Services, req: RenameDashboardRequest) -> Result<(), IpcError> {
    svc.dashboard_store.rename(req.id, req.title).await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(())
}

pub async fn delete_dashboard(svc: &Services, id: DashboardId) -> Result<(), IpcError> {
    svc.dashboard_store.delete(id).await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AddDashboardItemRequest {
    #[serde(rename = "dashboardId")]
    pub dashboard_id: DashboardId,
    /// `query` | `lens` | `text`.
    pub kind: String,
    /// A `query` tile's SQL.
    pub sql: Option<String>,
    /// A `query` tile's display: a lens viz, or `metric` (the metric card).
    pub display: Option<String>,
    /// A `lens` tile's lens id.
    #[serde(rename = "lensId")]
    pub lens_id: Option<String>,
    #[serde(rename = "optionsJson")]
    pub options_json: Option<String>,
}

pub async fn add_dashboard_item(
    svc: &Services,
    req: AddDashboardItemRequest,
) -> Result<DashboardItemId, IpcError> {
    let tile = oxplow_app::dashboard_tiles::new_tile(
        &svc.sql,
        oxplow_app::dashboard_tiles::TileInput {
            kind: req.kind,
            sql: req.sql,
            display: req.display,
            lens_id: req.lens_id,
            options_json: req.options_json,
        },
    )
    .await?;
    let id = svc.dashboard_store.add_item(req.dashboard_id, tile).await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(id)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateDashboardItemRequest {
    pub id: DashboardItemId,
    #[serde(rename = "optionsJson")]
    pub options_json: Option<String>,
}

pub async fn update_dashboard_item(
    svc: &Services,
    req: UpdateDashboardItemRequest,
) -> Result<(), IpcError> {
    // A query tile's SQL is held to the read contract on every edit too.
    if let Some(sql) = req
        .options_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| v.get("sql").and_then(|s| s.as_str()).map(str::to_string))
    {
        svc.sql.check(&sql).await?;
    }
    svc.dashboard_store
        .update_item(req.id, req.options_json)
        .await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(())
}

pub async fn remove_dashboard_item(svc: &Services, id: DashboardItemId) -> Result<(), IpcError> {
    svc.dashboard_store.remove_item(id).await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderDashboardItemsRequest {
    #[serde(rename = "dashboardId")]
    pub dashboard_id: DashboardId,
    pub order: Vec<DashboardItemId>,
}

pub async fn reorder_dashboard_items(
    svc: &Services,
    req: ReorderDashboardItemsRequest,
) -> Result<(), IpcError> {
    svc.dashboard_store
        .reorder_items(req.dashboard_id, req.order)
        .await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(())
}
