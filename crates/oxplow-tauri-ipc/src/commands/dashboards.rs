//! Tauri adapters for the `dashboards` command module (tsk138) — one-line
//! delegates to the `oxplow-rpc` cores. Request structs are re-exported so the
//! specta TS export sees them.

pub use oxplow_rpc::commands::dashboards::{
    AddDashboardItemRequest, DuplicateDashboardRequest, RenameDashboardRequest,
    ReorderDashboardItemsRequest, SetDashboardSettingsRequest, UpdateDashboardItemRequest,
};
