//! Per-function churn attribution now lives in `oxplow-rpc` (the
//! transport-neutral dispatch crate) so the `code_quality` cores can
//! use it without a `tauri` dependency. Re-exported here so existing
//! `crate::commands::churn::*` call sites keep working unchanged.
pub use oxplow_rpc::commands::churn::*;
