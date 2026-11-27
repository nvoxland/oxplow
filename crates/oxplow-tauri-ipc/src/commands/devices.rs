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
    pub settings: oxplow_host_access::settings::HostSettings,
    /// Whether the host runs with them now (they apply at the next start).
    pub settings_applied: bool,
    /// The addresses a device opens: `https://<name>:<port>/`.
    pub open_on_device: Vec<String>,
    /// The port other devices come in on and where it came from (the app's
    /// `--device-port` or `OXPLOW_DEVICE_PORT` win over the setting); none
    /// when the one given is no port (`port_problem` says).
    pub port: Option<oxplow_host_access::settings::DevicePort>,
    pub port_problem: Option<String>,
    /// Where the host listens for other devices now, or why it cannot.
    pub public: Option<oxplow_host::access::PublicListener>,
    /// Why the host did not answer: the settings still show, to fix.
    pub host_error: Option<String>,
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

/// The addresses a device opens for `settings` on `port` (the one in
/// force): each name (else the address) with the port, over HTTPS. Pure.
pub fn open_on_device(
    settings: &oxplow_host_access::settings::HostSettings,
    port: u16,
) -> Vec<String> {
    if !settings.allow_other_devices {
        return Vec::new();
    }
    let Ok(ip) = settings.address.trim().parse::<std::net::IpAddr>() else {
        return Vec::new();
    };
    // Other devices come in over HTTPS only.
    let scheme = "https";
    let names: Vec<String> = settings
        .names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let hosts = if names.is_empty() {
        vec![ip.to_string()]
    } else {
        names
    };
    hosts
        .into_iter()
        .map(|h| format!("{scheme}://{h}:{port}/"))
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
pub async fn host_devices(
    window: tauri::Window,
    app: tauri::AppHandle,
) -> Result<DevicesView, IpcError> {
    super::local_window_only(&window)?;
    let this_device = shell(&app)
        .device()
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "This computer".into());
    // The settings first: a host that does not answer (a setting that
    // cannot work, say) must not hide the form that fixes it.
    let settings = oxplow_host_access::settings::HostSettings::load(&config_dir()?);
    let answer = with_host(app.clone(), |host| async move {
        let (connected, public) = host.status_parts().await?;
        Ok((host.devices().await?, connected, public))
    })
    .await;
    let (devices, connected, public, host_error) = match answer {
        Ok((devices, connected, public)) => (devices, connected, Some(public), None),
        Err(e) => (Vec::new(), Vec::new(), None, Some(e.message)),
    };
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
    let (port, port_problem) = match settings.device_port() {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(DevicesView {
        this_device,
        devices,
        open_on_device: port
            .map(|p| open_on_device(&settings, p.port))
            .unwrap_or_default(),
        port,
        port_problem,
        // Applied: the host started with them and, when they let other
        // devices in, it listens for them.
        settings_applied: shell(&app).settings_applied(&settings)
            && host_error.is_none()
            && (!settings.allow_other_devices
                || matches!(
                    public,
                    Some(oxplow_host::access::PublicListener::Listening { .. })
                )),
        public,
        host_error,
        settings,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn host_pairing_code(
    window: tauri::Window,
    app: tauri::AppHandle,
) -> Result<PairingCodeView, IpcError> {
    super::local_window_only(&window)?;
    let (code, expires) = with_host(app, |host| async move { host.pairing_code().await }).await?;
    Ok(PairingCodeView {
        code,
        expires_in_seconds: i64::try_from(expires).unwrap_or(i64::MAX),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn host_rename_device(
    window: tauri::Window,
    app: tauri::AppHandle,
    rename: RenameDevice,
) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
    with_host(app, |host| async move {
        host.rename_device(&rename.id, &rename.name)
            .await
            .map(|_| ())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn host_remove_device(
    window: tauri::Window,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
    with_host(app, |host| async move { host.remove_device(&id).await }).await
}

/// Keep "Allow other devices"; it applies at the next app start.
#[tauri::command]
#[specta::specta]
pub async fn set_host_settings(
    window: tauri::Window,
    settings: oxplow_host_access::settings::HostSettings,
) -> Result<(), IpcError> {
    super::local_window_only(&window)?;
    settings.save(&config_dir()?).map_err(IpcError::invalid)
}

/// What the settings offer: this machine's addresses on private networks
/// (`oxplow_host_access::network::private_addresses`) and the name it answers to on
/// its local network (`<local hostname>.local`).
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostAddresses {
    pub ips: Vec<String>,
    pub local_name: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn host_addresses() -> Result<HostAddresses, IpcError> {
    Ok(HostAddresses {
        ips: oxplow_host_access::network::private_addresses()
            .into_iter()
            .map(|ip| ip.to_string())
            .collect(),
        local_name: oxplow_host_access::network::local_name(),
    })
}

/// The home's own root certificate (PEM), for the person to get onto a
/// device that cannot download it from the host; `None` before the host
/// first made one.
#[tauri::command]
#[specta::specta]
pub async fn host_root_certificate() -> Result<Option<String>, IpcError> {
    Ok(std::fs::read_to_string(oxplow_host_access::root_path(&config_dir()?)).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_host_access::settings::HostSettings;

    #[test]
    fn a_device_opens_each_name_with_the_port_over_https() {
        let on = HostSettings {
            allow_other_devices: true,
            address: "100.101.102.103".into(),
            names: vec!["mac.tailnet.ts.net".into()],
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
            port: None,
        };
        assert_eq!(
            open_on_device(&on, 7400),
            ["https://mac.tailnet.ts.net:7400/"]
        );
        assert_eq!(
            open_on_device(&on, 7401),
            ["https://mac.tailnet.ts.net:7401/"],
            "the port in force"
        );
        let own = HostSettings {
            names: vec![],
            tls_cert: None,
            tls_key: None,
            ..on.clone()
        };
        assert_eq!(
            open_on_device(&own, 7400),
            ["https://100.101.102.103:7400/"],
            "always HTTPS"
        );
        assert!(open_on_device(&HostSettings::default(), 7400).is_empty());
    }
}
