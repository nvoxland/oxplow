//! The Devices window: pairing a tablet or another computer with this Mac's
//! host, the paired devices, and "Allow other devices". Shell commands, so
//! they reach the app's own host as its owner (`ShellWindows::host_client`);
//! no page calls `/host/*` itself (`.context/host.md` "Pairing").

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;

use crate::error::IpcError;
use crate::windows::ShellWindows;

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    /// `desktop`, `tablet` or `phone`.
    pub kind: String,
    /// Seconds since the Unix epoch.
    #[specta(type = oxplow_domain::JsNumber)]
    pub paired_at: i64,
    /// It has a window open on this host now.
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DevicesView {
    /// This Mac's own name, as other devices see it.
    pub this_device: String,
    pub devices: Vec<DeviceView>,
    /// "Allow other devices", as saved.
    pub settings: oxplow_host::settings::HostSettings,
    /// Whether the host runs with them now (they apply at the next start).
    pub settings_applied: bool,
    /// The addresses a device opens: `https://<name>:<port>/`.
    pub open_on_device: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairingCodeView {
    /// `ABCD-EFGH`.
    pub code: String,
    #[specta(type = oxplow_domain::JsNumber)]
    pub expires_in_seconds: i64,
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameDevice {
    pub id: String,
    pub name: String,
}

/// The addresses a device opens for `settings`: each name (else the
/// address) with the port, `https` when there is a certificate. Pure.
pub fn open_on_device(settings: &oxplow_host::settings::HostSettings) -> Vec<String> {
    if !settings.allow_other_devices {
        return Vec::new();
    }
    let Ok(args) = settings.serve_args() else {
        return Vec::new();
    };
    let Some(bind) = args
        .iter()
        .position(|a| a == "--public-bind")
        .and_then(|i| args.get(i + 1))
        .and_then(|b| b.parse::<std::net::SocketAddr>().ok())
    else {
        return Vec::new();
    };
    let scheme = if settings.tls_cert.is_some() {
        "https"
    } else {
        "http"
    };
    let names: Vec<String> = settings
        .names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let hosts = if names.is_empty() {
        vec![bind.ip().to_string()]
    } else {
        names
    };
    hosts
        .into_iter()
        .map(|h| format!("{scheme}://{h}:{}/", bind.port()))
        .collect()
}

fn shell(app: &tauri::AppHandle) -> tauri::State<'_, ShellWindows> {
    app.state::<ShellWindows>()
}

/// Run `f` with the app's host (started if needed) off the main thread.
async fn with_host<T, F, Fut>(app: tauri::AppHandle, f: F) -> Result<T, IpcError>
where
    T: Send + 'static,
    F: FnOnce(oxplow_host::client::HostClient) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, oxplow_host::client::ClientError>> + Send,
{
    let host = tauri::async_runtime::spawn_blocking(move || shell(&app).host_client())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(IpcError::internal)?;
    f(host).await.map_err(|e| IpcError::invalid(e.to_string()))
}

fn config_dir() -> Result<std::path::PathBuf, IpcError> {
    oxplow_config::global_config_dir()
        .ok_or_else(|| IpcError::internal("no config folder for this user; set OXPLOW_HOME"))
}

#[tauri::command]
#[specta::specta]
pub async fn host_devices(app: tauri::AppHandle) -> Result<DevicesView, IpcError> {
    let this_device = shell(&app)
        .device()
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "This computer".into());
    let (devices, connected) = with_host(app.clone(), |host| async move {
        Ok((host.devices().await?, host.connected().await?))
    })
    .await?;
    let settings = oxplow_host::settings::HostSettings::load(&config_dir()?);
    let devices = devices
        .into_iter()
        .map(|d| DeviceView {
            connected: connected.iter().any(|c| c.device == d.id.as_str()),
            id: d.id.to_string(),
            name: d.name,
            kind: serde_json::to_value(d.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            paired_at: i64::try_from(d.paired_at).unwrap_or(i64::MAX),
        })
        .collect();
    Ok(DevicesView {
        this_device,
        devices,
        open_on_device: open_on_device(&settings),
        settings_applied: shell(&app).settings_applied(&settings),
        settings,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn host_pairing_code(app: tauri::AppHandle) -> Result<PairingCodeView, IpcError> {
    let (code, expires) = with_host(app, |host| async move { host.pairing_code().await }).await?;
    Ok(PairingCodeView {
        code,
        expires_in_seconds: i64::try_from(expires).unwrap_or(i64::MAX),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn host_rename_device(
    app: tauri::AppHandle,
    rename: RenameDevice,
) -> Result<(), IpcError> {
    with_host(app, |host| async move {
        host.rename_device(&rename.id, &rename.name)
            .await
            .map(|_| ())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn host_remove_device(app: tauri::AppHandle, id: String) -> Result<(), IpcError> {
    with_host(app, |host| async move { host.remove_device(&id).await }).await
}

/// Keep "Allow other devices"; it applies at the next app start.
#[tauri::command]
#[specta::specta]
pub async fn set_host_settings(
    settings: oxplow_host::settings::HostSettings,
) -> Result<(), IpcError> {
    settings.save(&config_dir()?).map_err(IpcError::invalid)
}

/// What Tailscale says about this machine, to fill the settings in.
#[tauri::command]
#[specta::specta]
pub async fn host_tailscale() -> Result<Option<oxplow_host::settings::TailscaleSelf>, IpcError> {
    tauri::async_runtime::spawn_blocking(oxplow_host::settings::ask_tailscale)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_host::settings::HostSettings;

    #[test]
    fn a_device_opens_each_name_with_the_port_over_https_with_a_certificate() {
        let on = HostSettings {
            allow_other_devices: true,
            address: "100.101.102.103".into(),
            names: vec!["mac.tailnet.ts.net".into()],
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
        };
        assert_eq!(open_on_device(&on), ["https://mac.tailnet.ts.net:7400/"]);
        let plain = HostSettings {
            names: vec![],
            tls_cert: None,
            tls_key: None,
            ..on.clone()
        };
        assert_eq!(open_on_device(&plain), ["http://100.101.102.103:7400/"]);
        assert!(open_on_device(&HostSettings::default()).is_empty());
    }
}
