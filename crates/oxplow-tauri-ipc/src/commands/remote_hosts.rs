//! The launcher's hosts on other machines: connect this Mac to a headless
//! host (`oxplow host serve` on a server) with a pairing code, list each
//! one's projects, open one, forget one (`oxplow_host::remote`,
//! `.context/host.md` "The Mac app").

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;

use crate::error::IpcError;
use crate::windows::ShellWindows;

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProjectView {
    pub id: String,
    pub name: String,
    /// `starting`, `running`, `stopping`, `stopped` or `crashed`.
    pub state: String,
    /// How many devices hold it now.
    #[specta(type = oxplow_domain::JsNumber)]
    pub holds: i64,
    pub held_by_you: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHostView {
    pub origin: String,
    /// What the host calls this Mac.
    pub device_name: String,
    /// Its version, when it answered.
    pub version: Option<String>,
    /// It runs another version than this app (said, never fixed here).
    pub other_version: bool,
    pub projects: Vec<RemoteProjectView>,
    /// Why it did not answer.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectHost {
    /// `server.tailnet.ts.net:7400`, `https://…`.
    pub address: String,
    pub code: String,
}

fn config_dir() -> Result<std::path::PathBuf, IpcError> {
    oxplow_config::global_config_dir()
        .ok_or_else(|| IpcError::internal("no config folder for this user; set OXPLOW_HOME"))
}

/// Pair this Mac with a host on another machine.
#[tauri::command]
#[specta::specta]
pub async fn connect_host(app: tauri::AppHandle, connect: ConnectHost) -> Result<(), IpcError> {
    let name = app
        .state::<ShellWindows>()
        .device()
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "Mac".into());
    oxplow_host::remote::pair(
        &config_dir()?,
        &oxplow_ai::secrets::KeychainSecrets,
        &connect.address,
        &connect.code,
        &name,
    )
    .await
    .map(|_| ())
    .map_err(IpcError::invalid)
}

/// Each paired host, with its projects (or why it did not answer).
#[tauri::command]
#[specta::specta]
pub async fn remote_hosts() -> Result<Vec<RemoteHostView>, IpcError> {
    let mut views = Vec::new();
    for host in oxplow_host::remote::list(&config_dir()?) {
        let answer = async {
            let client =
                oxplow_host::remote::client(&oxplow_ai::secrets::KeychainSecrets, &host.origin)?;
            let version = client.version().await.map_err(|e| e.to_string())?;
            let projects = client.projects().await.map_err(|e| e.to_string())?;
            Ok::<_, String>((version, projects))
        }
        .await;
        let view = match answer {
            Ok((version, projects)) => RemoteHostView {
                other_version: version != oxplow_host::VERSION,
                version: Some(version),
                projects: projects
                    .iter()
                    .map(|p| RemoteProjectView {
                        id: p["id"].as_str().unwrap_or_default().to_string(),
                        name: p["name"].as_str().unwrap_or_default().to_string(),
                        state: p["state"].as_str().unwrap_or_default().to_string(),
                        holds: p["holds"].as_i64().unwrap_or_default(),
                        held_by_you: p["held_by_you"].as_bool().unwrap_or_default(),
                    })
                    .collect(),
                error: None,
                origin: host.origin,
                device_name: host.device_name,
            },
            Err(error) => RemoteHostView {
                origin: host.origin,
                device_name: host.device_name,
                version: None,
                other_version: false,
                projects: Vec::new(),
                error: Some(error),
            },
        };
        views.push(view);
    }
    Ok(views)
}

/// Open a project of a host on another machine in a window of its own;
/// `close_caller` closes the window that asked (the launcher), as an open
/// from it does.
#[tauri::command]
#[specta::specta]
pub async fn open_remote_project(
    window: tauri::Window,
    origin: String,
    project: String,
    close_caller: bool,
) -> Result<(), IpcError> {
    let app = window.app_handle().clone();
    let opened = tauri::async_runtime::spawn_blocking(move || {
        app.state::<ShellWindows>()
            .open_remote(&app, &origin, &project)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(IpcError::invalid)?;
    if close_caller && opened != window.label() {
        window
            .close()
            .map_err(|e| IpcError::internal(format!("close window: {e}")))?;
    }
    Ok(())
}

/// Forget a host on another machine: this Mac's device goes there, and the
/// token here.
#[tauri::command]
#[specta::specta]
pub async fn forget_remote_host(origin: String) -> Result<(), IpcError> {
    oxplow_host::remote::forget(
        &config_dir()?,
        &oxplow_ai::secrets::KeychainSecrets,
        &origin,
    )
    .await
    .map_err(IpcError::invalid)
}
