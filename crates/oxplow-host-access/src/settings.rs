//! "Allow other devices": how the Mac app's host lets a tablet or another
//! computer reach it. The shell's own file in the home, `host-settings.json`
//! (beside `device.json` and `session.json`), edited in the Devices window,
//! read when the app starts its host, so a change applies at the next start.
//!
//! Off: the host listens on loopback only. On: a second listener, HTTPS
//! only, on the address the person chose from this machine's (never every
//! network by default), with the names a device reaches it by for the
//! `Host` check. Its certificate is the home's own (`crate::certs`, which a
//! device trusts once), or one of the person's signed by any CA.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The default port for other devices: fixed, because a browser's origin is
/// its identity (and holds its token), and each app start is a host start.
pub const DEFAULT_PORT: u16 = 7400;

/// The environment variable that names the port for other devices, over the
/// saved setting (and under a `--device-port` flag).
pub const PORT_ENV: &str = "OXPLOW_DEVICE_PORT";

/// Where the port for other devices came from, for the Devices window to say
/// (a flag or the environment wins over what it saves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PortFrom {
    /// `--device-port` on the app's or the host's command line.
    Flag,
    /// `OXPLOW_DEVICE_PORT`.
    Env,
    /// The saved setting.
    Saved,
    Default,
}

/// The port other devices come in on, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DevicePort {
    pub port: u16,
    pub from: PortFrom,
}

/// The port for other devices: `flag` (`--device-port`), else `env`
/// (`OXPLOW_DEVICE_PORT`, blank is none), else `saved`, else
/// [`DEFAULT_PORT`]. Pure.
pub fn device_port(
    flag: Option<u16>,
    env: Option<&str>,
    saved: Option<u16>,
) -> Result<DevicePort, SettingsError> {
    if let Some(port) = flag {
        return Ok(DevicePort {
            port,
            from: PortFrom::Flag,
        });
    }
    if let Some(text) = env.map(str::trim).filter(|t| !t.is_empty()) {
        return match text.parse::<u16>() {
            Ok(port) if port > 0 => Ok(DevicePort {
                port,
                from: PortFrom::Env,
            }),
            _ => Err(SettingsError::BadPort(text.to_string())),
        };
    }
    Ok(match saved {
        Some(port) => DevicePort {
            port,
            from: PortFrom::Saved,
        },
        None => DevicePort {
            port: DEFAULT_PORT,
            from: PortFrom::Default,
        },
    })
}

/// The app's `--device-port`, kept once when it starts.
static PORT_FLAG: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// Keep the app's `--device-port` (its command line), for every host start.
pub fn set_port_flag(port: u16) {
    let _ = PORT_FLAG.set(port);
}

/// `--device-port` parsed: a port of 1 to 65535.
pub fn parse_port(text: &str) -> Result<u16, SettingsError> {
    match text.trim().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(SettingsError::BadPort(text.to_string())),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSettings {
    pub allow_other_devices: bool,
    /// Where other devices reach this machine: an IP address of its own
    /// (`100.101.102.103`). The port is `port`.
    pub address: String,
    /// Names a device may use in its address bar besides the address
    /// (`studio.local`). With the host's own certificate, private names only.
    pub names: Vec<String>,
    /// A certificate of the person's own (PEM, signed by any CA) in place of
    /// the host's.
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    /// The port other devices come in on; none is [`DEFAULT_PORT`]. A
    /// `--device-port` flag or `OXPLOW_DEVICE_PORT` wins over it.
    pub port: Option<u16>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("other devices need an address to reach this computer by")]
    NoAddress,
    #[error(
        "\"{0}\" is not an address (write it like 100.101.102.103; the port is its own setting)"
    )]
    BadAddress(String),
    #[error("\"{0}\" is not a port (1 to 65535)")]
    BadPort(String),
    #[error("a certificate of your own needs both the certificate and its key")]
    HalfTls,
    #[error("{0} is no private name, so the host's own certificate cannot cover it; give a certificate of your own")]
    PublicName(String),
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-settings.json")
}

impl HostSettings {
    /// The home's settings; none saved (or a file that cannot be read) is
    /// off.
    pub fn load(config_dir: &Path) -> Self {
        std::fs::read(path(config_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Check and keep them.
    pub fn save(&self, config_dir: &Path) -> Result<(), String> {
        // What it keeps is checked, on its own port (an override in the
        // environment is no part of it).
        let own = device_port(None, None, self.port).map_err(|e| e.to_string())?;
        self.serve_args_with(own).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        oxplow_config::write_atomic(&path(config_dir), &bytes).map_err(|e| e.to_string())
    }

    /// The port other devices come in on now: the app's `--device-port`,
    /// else `OXPLOW_DEVICE_PORT`, else the saved one, else the default.
    pub fn device_port(&self) -> Result<DevicePort, SettingsError> {
        let env = std::env::var(PORT_ENV).ok();
        device_port(PORT_FLAG.get().copied(), env.as_deref(), self.port)
    }

    /// The addresses a device opens on `port` (the one in force): each
    /// name (else the address) with the port, over HTTPS (other devices
    /// come in over HTTPS only). None while other devices are off. Pure.
    pub fn open_on_device(&self, port: u16) -> Vec<String> {
        if !self.allow_other_devices {
            return Vec::new();
        }
        let Ok(ip) = self.address.trim().parse::<std::net::IpAddr>() else {
            return Vec::new();
        };
        let names: Vec<String> = self
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
            .map(|h| format!("https://{h}:{port}/"))
            .collect()
    }

    /// The `serve` flags these settings add, on the port in force: none
    /// when off.
    pub fn serve_args(&self) -> Result<Vec<String>, SettingsError> {
        if !self.allow_other_devices {
            return Ok(Vec::new());
        }
        self.serve_args_with(self.device_port()?)
    }

    /// The `serve` flags these settings add on `port`: none when off. Pure.
    pub fn serve_args_with(&self, port: DevicePort) -> Result<Vec<String>, SettingsError> {
        if !self.allow_other_devices {
            return Ok(Vec::new());
        }
        let address = self.address.trim();
        if address.is_empty() {
            return Err(SettingsError::NoAddress);
        }
        let Ok(ip) = address.parse::<std::net::IpAddr>() else {
            return Err(SettingsError::BadAddress(address.to_string()));
        };
        // The address is a name a device may use too (`https://<ip>:<port>/`).
        let mut args = vec![
            "--public-bind".to_string(),
            ip.to_string(),
            "--device-port".into(),
            port.port.to_string(),
            "--allow-host".into(),
            ip.to_string(),
        ];
        for name in self
            .names
            .iter()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
        {
            args.push("--allow-host".into());
            args.push(name.to_string());
        }
        match (&self.tls_cert, &self.tls_key) {
            (None, None) => {
                if let Some(public) = self
                    .names
                    .iter()
                    .map(|n| n.trim())
                    .find(|n| !n.is_empty() && !crate::network::is_private_name(n))
                {
                    return Err(SettingsError::PublicName(public.to_string()));
                }
            }
            (Some(cert), Some(key)) => {
                args.push("--tls-cert".into());
                args.push(cert.display().to_string());
                args.push("--tls-key".into());
                args.push(key.display().to_string());
            }
            _ => return Err(SettingsError::HalfTls),
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_adds_nothing_and_on_names_the_listener_its_names_and_its_certificate() {
        assert_eq!(HostSettings::default().serve_args(), Ok(vec![]));
        let on = HostSettings {
            allow_other_devices: true,
            address: "100.101.102.103".into(),
            names: vec!["mac.tailnet.ts.net".into(), " ".into()],
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
            port: None,
        };
        assert_eq!(
            on.serve_args_with(DevicePort {
                port: 7400,
                from: PortFrom::Default
            })
            .unwrap(),
            [
                "--public-bind",
                "100.101.102.103",
                "--device-port",
                "7400",
                "--allow-host",
                "100.101.102.103",
                "--allow-host",
                "mac.tailnet.ts.net",
                "--tls-cert",
                "/c.pem",
                "--tls-key",
                "/k.pem"
            ]
        );
        let args = on
            .serve_args_with(DevicePort {
                port: 8443,
                from: PortFrom::Env,
            })
            .unwrap();
        assert_eq!(args[3], "8443");
    }

    #[test]
    fn a_device_opens_each_name_with_the_port_over_https() {
        let on = HostSettings {
            allow_other_devices: true,
            address: "100.101.102.103".into(),
            names: vec!["mac.tailnet.ts.net".into()],
            ..Default::default()
        };
        assert_eq!(
            on.open_on_device(7400),
            ["https://mac.tailnet.ts.net:7400/"]
        );
        assert_eq!(
            on.open_on_device(7401),
            ["https://mac.tailnet.ts.net:7401/"],
            "the port in force"
        );
        let own = HostSettings {
            names: vec![],
            ..on
        };
        assert_eq!(own.open_on_device(7400), ["https://100.101.102.103:7400/"]);
        assert!(HostSettings::default().open_on_device(7400).is_empty());
    }

    /// The port other devices come in on: the app's or host's flag, else
    /// `OXPLOW_DEVICE_PORT`, else the saved setting, else 7400; each says
    /// where it came from, and one that is no port is said.
    #[test]
    fn the_device_port_is_the_flag_then_the_env_then_the_setting() {
        let got = |flag, env, saved| device_port(flag, env, saved);
        assert_eq!(
            got(Some(7500), Some("7600"), Some(7700)),
            Ok(DevicePort {
                port: 7500,
                from: PortFrom::Flag
            })
        );
        assert_eq!(
            got(None, Some("7600"), Some(7700)),
            Ok(DevicePort {
                port: 7600,
                from: PortFrom::Env
            })
        );
        assert_eq!(
            got(None, Some(" "), Some(7700)),
            Ok(DevicePort {
                port: 7700,
                from: PortFrom::Saved
            })
        );
        assert_eq!(
            got(None, None, None),
            Ok(DevicePort {
                port: DEFAULT_PORT,
                from: PortFrom::Default
            })
        );
        assert_eq!(
            got(None, Some("seventy"), None),
            Err(SettingsError::BadPort("seventy".into()))
        );
        assert_eq!(
            got(None, Some("0"), None),
            Err(SettingsError::BadPort("0".into()))
        );
    }

    #[test]
    fn what_cannot_work_is_said_and_not_kept() {
        let home = tempfile::tempdir().unwrap();
        let on = |address: &str| HostSettings {
            allow_other_devices: true,
            address: address.into(),
            ..Default::default()
        };
        assert_eq!(on("").serve_args(), Err(SettingsError::NoAddress));
        assert!(matches!(
            on("mac.local").serve_args(),
            Err(SettingsError::BadAddress(_))
        ));
        // The port is its own setting: an address is one address.
        assert!(matches!(
            on("100.64.0.1:7400").serve_args(),
            Err(SettingsError::BadAddress(_))
        ));
        let half = HostSettings {
            tls_cert: Some("/c.pem".into()),
            ..on("100.64.0.1")
        };
        assert_eq!(half.serve_args(), Err(SettingsError::HalfTls));
        // The host's own certificate covers private names only.
        let named = |name: &str| HostSettings {
            names: vec![name.into()],
            ..on("192.168.1.20")
        };
        assert!(named("studio.local").serve_args().is_ok());
        assert_eq!(
            named("mac.example.com").serve_args(),
            Err(SettingsError::PublicName("mac.example.com".into()))
        );
        assert!(half.save(home.path()).is_err());
        assert_eq!(
            HostSettings::load(home.path()),
            HostSettings::default(),
            "nothing kept"
        );

        on("100.64.0.1").save(home.path()).unwrap();
        assert_eq!(HostSettings::load(home.path()), on("100.64.0.1"));
    }
}
