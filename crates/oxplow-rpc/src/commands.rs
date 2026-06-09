//! Command "core" functions: the real request-handling bodies, lifted
//! out of the Tauri `#[tauri::command]` wrappers so they can be called
//! both locally (oxplow-tauri-ipc) and remotely (oxplow-daemon HTTP).
//!
//! Convention: every core takes `svc: &Services` as its first parameter
//! (even when unused) so the [`crate::dispatch`] registry can call them
//! uniformly. Remaining parameters mirror the original command's args
//! one-for-one, in declaration order.
//!
//! This module is grown incrementally as commands are migrated off the
//! Tauri-only path; see the registry in [`crate::dispatch`].

use oxplow_app::Services;
use oxplow_domain::stores::TaskStore;
use oxplow_domain::{Stream, Task, TaskId};

use crate::error::IpcError;

/// Liveness check the UI uses to verify the daemon is reachable.
pub async fn ping(_svc: &Services) -> Result<&'static str, IpcError> {
    Ok("pong")
}

pub async fn list_streams(svc: &Services) -> Result<Vec<Stream>, IpcError> {
    Ok(svc.streams.list_streams().await?)
}

pub async fn get_task(svc: &Services, id: TaskId) -> Result<Option<Task>, IpcError> {
    Ok(svc.task_store.get(id).await?)
}
