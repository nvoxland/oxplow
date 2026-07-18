//! Cores for the `dashboards` command module (epic tsk138) — user-created
//! dashboards of metric tiles. Project-global; every write emits
//! `OxplowEvent::DashboardsChanged` so agent- and UI-driven edits both
//! live-refresh the renderer.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::{OxplowEvent, Services};
use oxplow_db::{Dashboard, DashboardWithItems, NewDashboardItem};
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
    /// `metric` | `text`.
    pub kind: String,
    #[serde(rename = "metricKey")]
    pub metric_key: Option<String>,
    #[serde(rename = "optionsJson")]
    pub options_json: Option<String>,
}

pub async fn add_dashboard_item(
    svc: &Services,
    req: AddDashboardItemRequest,
) -> Result<DashboardItemId, IpcError> {
    let id = svc
        .dashboard_store
        .add_item(
            req.dashboard_id,
            NewDashboardItem {
                kind: req.kind,
                metric_key: req.metric_key,
                options_json: req.options_json,
            },
        )
        .await?;
    svc.events.emit(OxplowEvent::DashboardsChanged);
    Ok(id)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateDashboardItemRequest {
    pub id: DashboardItemId,
    #[serde(rename = "metricKey")]
    pub metric_key: Option<String>,
    #[serde(rename = "optionsJson")]
    pub options_json: Option<String>,
}

pub async fn update_dashboard_item(
    svc: &Services,
    req: UpdateDashboardItemRequest,
) -> Result<(), IpcError> {
    svc.dashboard_store
        .update_item(req.id, req.metric_key, req.options_json)
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
