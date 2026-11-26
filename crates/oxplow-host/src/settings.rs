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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSettings {
    pub allow_other_devices: bool,
    /// Where other devices reach this machine: `100.101.102.103:7400`, or a
    /// bare address (the default port is added).
    pub address: String,
    /// Names a device may use in its address bar besides the address
    /// (`studio.local`). With the host's own certificate, private names only.
    pub names: Vec<String>,
    /// A certificate of the person's own (PEM, signed by any CA) in place of
    /// the host's.
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("other devices need an address to reach this computer by")]
    NoAddress,
    #[error("\"{0}\" is not an address (write it like 100.101.102.103 or 100.101.102.103:7400)")]
    BadAddress(String),
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
            (None, None) => {
                if let Some(public) = self
                    .names
                    .iter()
                    .map(|n| n.trim())
                    .find(|n| !n.is_empty() && !crate::certs::is_private_name(n))
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
