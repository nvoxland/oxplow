//! The launcher's hosts on other machines: pair this Mac with a headless
//! host (`oxplow host serve` on a server: this Mac asks and shows a code,
//! the person there types it), list each
//! one's projects, open one, forget one (`oxplow_host::remote`,
//! `.context/host.md` "The Mac app").

use serde::Serialize;
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

/// This Mac's ask to pair with a host on another computer: the code for
/// the person there to type, and what collects the answer.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostAsk {
    pub origin: String,
    /// The ask's secret: what `collect_host` takes.
    pub request: String,
    /// `ABCD-EFGH`: typed there (`oxplow host pair <code>`, or its Devices
    /// page).
    pub code: String,
    /// Seconds it waits.
    #[specta(type = oxplow_domain::JsNumber)]
    pub expires_in: i64,
}

fn config_dir() -> Result<std::path::PathBuf, IpcError> {
    oxplow_config::global_config_dir()
        .ok_or_else(|| IpcError::internal("no config folder for this user; set OXPLOW_HOME"))
}

/// Ask a host on another computer (`server.tailnet.ts.net:7400`,
/// `https://…`) to pair this Mac: the code to type there.
#[tauri::command]
#[specta::specta]
pub async fn ask_host(
    window: tauri::Window,
    app: tauri::AppHandle,
    address: String,
) -> Result<HostAsk, IpcError> {
    super::local_window_only(&window)?;
    let name = app
        .state::<ShellWindows>()
        .device()
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "Mac".into());
    let (origin, ask) = oxplow_host::remote::ask(&address, &name)
        .await
        .map_err(IpcError::invalid)?;
    Ok(HostAsk {
        origin,
        request: ask.request,
        code: ask.code,
        expires_in: ask.expires_in as i64,
    })
}

/// Wait (a while) for the person at that host to type the code: `true`
/// once they did (this Mac is paired, its token kept), `false` while it
/// still waits; an error once the ask ended.
#[tauri::command]
#[specta::specta]
pub async fn collect_host(
    window: tauri::Window,
    origin: String,
    request: String,
) -> Result<bool, IpcError> {
    super::local_window_only(&window)?;
    oxplow_host::remote::collect(
        &config_dir()?,
        &oxplow_ai::secrets::KeychainSecrets,
        &origin,
        &request,
    )
    .await
    .map(|host| host.is_some())
    .map_err(IpcError::invalid)
}

/// Each paired host, with its projects (or why it did not answer).
#[tauri::command]
#[specta::specta]
pub async fn remote_hosts() -> Result<Vec<RemoteHostView>, IpcError> {
    // All at once: one host that does not answer holds up no other.
    let asked: Vec<_> = oxplow_host::remote::list(&config_dir()?)
        .into_iter()
        .map(|host| tauri::async_runtime::spawn(view_of(host)))
        .collect();
    let mut views = Vec::new();
    for task in asked {
        views.push(task.await.map_err(|e| IpcError::internal(e.to_string()))?);
    }
    Ok(views)
}

/// One paired host, with its projects, or why it did not answer.
async fn view_of(host: oxplow_host::remote::RemoteHost) -> RemoteHostView {
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
    view
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
    super::local_window_only(&window)?;
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
pub async fn forget_remote_host(window: tauri::Window, origin: String) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
    oxplow_host::remote::forget(
        &config_dir()?,
        &oxplow_ai::secrets::KeychainSecrets,
        &origin,
    )
    .await
    .map_err(IpcError::invalid)?;
    window
        .app_handle()
        .state::<ShellWindows>()
        .forget_unrestored(&origin);
    Ok(())
}
