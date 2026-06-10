//! Cores for the `background` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::background_task::{StartInput, UpdateInput};
use oxplow_app::Services;
use oxplow_app::{BackgroundTask, BackgroundTaskKind};

use crate::error::IpcError;

pub async fn list_background_tasks(svc: &Services) -> Result<Vec<BackgroundTask>, IpcError> {
    Ok(svc.background_tasks.list_running())
}

pub async fn get_background_task(
    svc: &Services,
    id: String,
) -> Result<Option<BackgroundTask>, IpcError> {
    Ok(svc.background_tasks.get(&id))
}

pub async fn start_background_task(
    svc: &Services,
    kind: BackgroundTaskKind,
    label: String,
    detail: Option<String>,
) -> Result<BackgroundTask, IpcError> {
    Ok(svc.background_tasks.start(StartInput {
        kind,
        label,
        detail,
        progress: None,
    }))
}

pub async fn complete_background_task(
    svc: &Services,
    id: String,
    result_json: Option<String>,
) -> Result<(), IpcError> {
    let result = result_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    svc.background_tasks.complete(&id, result);
    Ok(())
}

pub async fn fail_background_task(
    svc: &Services,
    id: String,
    error: String,
) -> Result<(), IpcError> {
    svc.background_tasks.fail(&id, error, None);
    Ok(())
}

pub async fn update_background_task(
    svc: &Services,
    id: String,
    label: Option<String>,
    detail: Option<Option<String>>,
    progress: Option<Option<f64>>,
) -> Result<(), IpcError> {
    svc.background_tasks.update(
        &id,
        UpdateInput {
            label,
            detail,
            progress,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_background_tasks_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_background_tasks", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_array(), "expected a JSON array, got {out}");
    }

    #[tokio::test]
    async fn get_background_task_returns_null_for_missing() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_background_task",
            serde_json::json!({ "id": "nope" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out, serde_json::json!(null));
    }
}
