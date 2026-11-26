//! "Allow other devices": how the Mac app's host lets a tablet or another
//! computer reach it. The shell's own file in the home, `host-settings.json`
//! (beside `device.json` and `session.json`), edited in the Devices window,
//! read when the app starts its host, so a change applies at the next start.
//!
//! Off: the host listens on loopback only. On: a second listener on the
//! address the person chose (never every network by default: the Mac's
//! Tailscale address, say), with the names a device reaches it by for the
//! `Host` check, and HTTPS when a certificate and key are given.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The default port for other devices: fixed, because a browser's origin is
/// its identity (and holds its token), and each app start is a host start.
pub const DEFAULT_PORT: u16 = 7400;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSettings {
    pub allow_other_devices: bool,
    /// Where other devices reach this machine: `100.101.102.103:7400`, or a
    /// bare address (the default port is added).
    pub address: String,
    /// The names a device uses in its address bar (`mac.tailnet.ts.net`).
    pub names: Vec<String>,
    /// PEM files for HTTPS (`tailscale cert <name>` writes both).
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("other devices need an address to reach this computer by")]
    NoAddress,
    #[error("\"{0}\" is not an address (write it like 100.101.102.103 or 100.101.102.103:7400)")]
    BadAddress(String),
    #[error("HTTPS needs both the certificate and its key")]
    HalfTls,
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
        self.serve_args().map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        oxplow_config::write_atomic(&path(config_dir), &bytes).map_err(|e| e.to_string())
    }

    /// The `serve` flags these settings add: none when off.
    pub fn serve_args(&self) -> Result<Vec<String>, SettingsError> {
        if !self.allow_other_devices {
            return Ok(Vec::new());
        }
        let address = self.address.trim();
        if address.is_empty() {
            return Err(SettingsError::NoAddress);
        }
        let bind = if address.parse::<std::net::SocketAddr>().is_ok() {
            address.to_string()
        } else if let Ok(ip) = address.parse::<std::net::IpAddr>() {
            std::net::SocketAddr::new(ip, DEFAULT_PORT).to_string()
        } else {
            return Err(SettingsError::BadAddress(address.to_string()));
        };
        // The address is a name a device may use too (`http://<ip>:7400/`).
        let ip = bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();
        let mut args = vec!["--public-bind".to_string(), bind, "--allow-host".into(), ip];
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
            (None, None) => {}
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

/// What Tailscale says about this machine, to fill the settings in: its
/// tailnet address and its MagicDNS name (`tailscale cert` makes a
/// certificate for that name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleSelf {
    pub address: String,
    pub name: Option<String>,
}

/// This machine from `tailscale status --json` output. Pure.
pub fn tailscale_self(status_json: &str) -> Option<TailscaleSelf> {
    let status: serde_json::Value = serde_json::from_str(status_json).ok()?;
    let me = &status["Self"];
    let address = me["TailscaleIPs"]
        .as_array()?
        .iter()
        .filter_map(|ip| ip.as_str())
        .find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())?
        .to_string();
    let name = me["DNSName"]
        .as_str()
        .map(|n| n.trim_end_matches('.').to_string())
        .filter(|n| !n.is_empty());
    Some(TailscaleSelf { address, name })
}

/// Ask the Tailscale CLI (on the path, or the Mac app's) about this machine;
/// `None` without Tailscale or when it is not up. **Blocking.**
pub fn ask_tailscale() -> Option<TailscaleSelf> {
    [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]
    .into_iter()
    .find_map(|cli| {
        let out = std::process::Command::new(cli)
            .args(["status", "--json"])
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    })
    .and_then(|stdout| tailscale_self(&String::from_utf8_lossy(&stdout)))
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
        };
        assert_eq!(
            on.serve_args().unwrap(),
            [
                "--public-bind",
                "100.101.102.103:7400",
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
        let with_port = HostSettings {
            address: "100.101.102.103:8443".into(),
            tls_cert: None,
            tls_key: None,
            ..on
        };
        assert_eq!(with_port.serve_args().unwrap()[1], "100.101.102.103:8443");
    }

    #[test]
    fn tailscale_says_this_machines_address_and_name() {
        let json = r#"{"Self":{"DNSName":"mac.tailnet.ts.net.","TailscaleIPs":["fd7a:115c::1","100.101.102.103"]}}"#;
        assert_eq!(
            tailscale_self(json),
            Some(TailscaleSelf {
                address: "100.101.102.103".into(),
                name: Some("mac.tailnet.ts.net".into()),
            })
        );
        assert_eq!(tailscale_self(r#"{"Self":{}}"#), None);
        assert_eq!(tailscale_self("not json"), None);
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
        let half = HostSettings {
            tls_cert: Some("/c.pem".into()),
            ..on("100.64.0.1")
        };
        assert_eq!(half.serve_args(), Err(SettingsError::HalfTls));
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
