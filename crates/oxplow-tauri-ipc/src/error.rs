//! The frontend-facing error envelope now lives in `oxplow-rpc` (the
//! transport-neutral dispatch crate) so the headless daemon can share it
//! without depending on `tauri`. Re-exported here so the many
//! `use crate::error::IpcError;` call sites keep working unchanged.
pub use oxplow_rpc::IpcError;
