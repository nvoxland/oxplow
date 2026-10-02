//! Reads for user-created dashboards (epic tsk138). Every write is a
//! `dashboard.*` command on the bus (P8.A5); views re-read when
//! `v_dashboard` / `v_dashboard_item` change.

use oxplow_app::Services;
use oxplow_db::{Dashboard, DashboardWithItems};
use oxplow_domain::DashboardId;

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
