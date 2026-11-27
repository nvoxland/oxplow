//! "Allow other devices": how the host lets a tablet or another computer
//! reach it. The host keeps them (`host.db`) and applies a change at once;
//! the Devices page reads and changes them through the host. What `serve`
//! was told for one run (its flags, `OXPLOW_DEVICE_PORT`) replaces what is
//! saved for that run and is never kept ([`Overrides`], [`effective`]).
//!
//! Off: the host listens on loopback only. On: a second listener, HTTPS
//! only, on the address the person chose from this machine's (never every
//! network by default), with the names a device reaches it by for the
//! `Host` check. Its certificate is the home's own (which a device trusts
//! once), or one of the person's signed by any CA.

use std::net::IpAddr;
use std::path::PathBuf;

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

/// The app's `--device-port`, when it was started with one.
pub fn port_flag() -> Option<u16> {
    PORT_FLAG.get().copied()
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
    /// Other names a device may use in its address bar besides the address,
    /// ones the person's network gives this machine (`mac.tailnet.ts.net`).
    /// With the host's own certificate, private names only; its own `.local`
    /// name the host adds itself (`network::with_local_name`).
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

impl SettingsError {
    /// The setting it is about (`HostSettings`' field, as the form names it).
    pub fn field(&self) -> &'static str {
        match self {
            SettingsError::NoAddress | SettingsError::BadAddress(_) => "address",
            SettingsError::BadPort(_) => "port",
            SettingsError::HalfTls => "tls_cert",
            SettingsError::PublicName(_) => "names",
        }
    }
}

fn tidy_names(names: &[String]) -> Vec<String> {
    names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// A name the host's own certificate cannot cover, when it is the one used.
fn public_name(names: &[String], own_tls: bool) -> Result<(), SettingsError> {
    match names
        .iter()
        .find(|n| !own_tls && !crate::network::is_private_name(n))
    {
        Some(public) => Err(SettingsError::PublicName(public.clone())),
        None => Ok(()),
    }
}

impl HostSettings {
    /// These settings as they are kept: names trimmed (blanks dropped), and
    /// what cannot work refused. Off checks only what it would keep
    /// regardless (the port, a certificate with its key). Pure.
    pub fn checked(self) -> Result<Self, SettingsError> {
        let names = tidy_names(&self.names);
        let address = self.address.trim().to_string();
        if self.port == Some(0) {
            return Err(SettingsError::BadPort("0".into()));
        }
        let own_tls = match (&self.tls_cert, &self.tls_key) {
            (None, None) => false,
            (Some(_), Some(_)) => true,
            _ => return Err(SettingsError::HalfTls),
        };
        if self.allow_other_devices {
            if address.is_empty() {
                return Err(SettingsError::NoAddress);
            }
            if address.parse::<IpAddr>().is_err() {
                return Err(SettingsError::BadAddress(address));
            }
            public_name(&names, own_tls)?;
        }
        Ok(Self {
            names,
            address,
            ..self
        })
    }
}

/// What `serve` was told for this run, over what is saved: never kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `--public-bind <ip>`: on, at this address.
    pub address: Option<IpAddr>,
    /// `--device-port <port>`.
    pub port: Option<u16>,
    /// `OXPLOW_DEVICE_PORT`, as the environment has it.
    pub port_env: Option<String>,
    /// `--allow-host <name>`…: these names in place of the saved ones.
    pub names: Option<Vec<String>>,
    /// `--tls-cert <pem> --tls-key <pem>`.
    pub tls: Option<(PathBuf, PathBuf)>,
}

/// The settings in force: what is saved, with each override in its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub allow_other_devices: bool,
    pub address: String,
    pub port: DevicePort,
    pub names: Vec<String>,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    /// The flags (and the environment variable) in force over what is
    /// saved, as they are written: `--public-bind`, `--device-port`,
    /// `OXPLOW_DEVICE_PORT`, `--allow-host`, `--tls-cert`.
    pub overridden: Vec<String>,
}

/// The settings in force for `saved` under this run's `overrides`, checked
/// as a saved one is. Pure.
pub fn effective(saved: &HostSettings, overrides: &Overrides) -> Result<Effective, SettingsError> {
    let mut overridden = Vec::new();
    let mut said = |flag: &str, given: bool| {
        if given {
            overridden.push(flag.to_string());
        }
    };
    said("--public-bind", overrides.address.is_some());
    let port = device_port(overrides.port, overrides.port_env.as_deref(), saved.port)?;
    said("--device-port", port.from == PortFrom::Flag);
    said(PORT_ENV, port.from == PortFrom::Env);
    said("--allow-host", overrides.names.is_some());
    said("--tls-cert", overrides.tls.is_some());
    let (tls_cert, tls_key) = match &overrides.tls {
        Some((cert, key)) => (Some(cert.clone()), Some(key.clone())),
        None => (saved.tls_cert.clone(), saved.tls_key.clone()),
    };
    let in_force = HostSettings {
        allow_other_devices: saved.allow_other_devices || overrides.address.is_some(),
        address: overrides
            .address
            .map_or_else(|| saved.address.clone(), |ip| ip.to_string()),
        names: overrides
            .names
            .clone()
            .unwrap_or_else(|| saved.names.clone()),
        tls_cert,
        tls_key,
        // The port is decided above (a flag's 0 is a free port, a test's).
        port: None,
    }
    .checked()?;
    Ok(Effective {
        allow_other_devices: in_force.allow_other_devices,
        address: in_force.address,
        port,
        names: in_force.names,
        tls_cert: in_force.tls_cert,
        tls_key: in_force.tls_key,
        overridden,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(address: &str) -> HostSettings {
        HostSettings {
            allow_other_devices: true,
            address: address.into(),
            ..Default::default()
        }
    }

    /// The saved settings, checked as they are kept: names tidied, and
    /// what cannot work said with the field it is about.
    #[test]
    fn what_is_kept_is_tidied_and_what_cannot_work_is_said() {
        assert_eq!(
            HostSettings::default().checked(),
            Ok(HostSettings::default())
        );
        // Off checks nothing it would not use.
        let off = HostSettings {
            address: "not an address".into(),
            ..Default::default()
        };
        assert!(off.checked().is_ok());
        let tidy = HostSettings {
            names: vec![" studio.local ".into(), " ".into()],
            ..on(" 192.168.1.20 ")
        };
        let kept = tidy.checked().unwrap();
        assert_eq!(kept.address, "192.168.1.20");
        assert_eq!(kept.names, ["studio.local"]);

        assert_eq!(on("").checked(), Err(SettingsError::NoAddress));
        assert!(matches!(
            on("mac.local").checked(),
            Err(SettingsError::BadAddress(_))
        ));
        // The port is its own setting: an address is one address.
        assert!(matches!(
            on("100.64.0.1:7400").checked(),
            Err(SettingsError::BadAddress(_))
        ));
        let zero = HostSettings {
            port: Some(0),
            ..on("100.64.0.1")
        };
        assert_eq!(zero.checked(), Err(SettingsError::BadPort("0".into())));
        let half = HostSettings {
            tls_cert: Some("/c.pem".into()),
            ..on("100.64.0.1")
        };
        assert_eq!(half.checked(), Err(SettingsError::HalfTls));
        // The host's own certificate covers private names only.
        let named = |name: &str| HostSettings {
            names: vec![name.into()],
            ..on("192.168.1.20")
        };
        assert!(named("studio.local").checked().is_ok());
        assert_eq!(
            named("mac.example.com").checked(),
            Err(SettingsError::PublicName("mac.example.com".into()))
        );
        let own = HostSettings {
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
            ..named("mac.example.com")
        };
        assert!(
            own.checked().is_ok(),
            "a certificate of your own covers any name"
        );

        assert_eq!(SettingsError::NoAddress.field(), "address");
        assert_eq!(SettingsError::BadPort("x".into()).field(), "port");
        assert_eq!(SettingsError::PublicName("x".into()).field(), "names");
        assert_eq!(SettingsError::HalfTls.field(), "tls_cert");
    }

    /// What `serve` was told for this run replaces what is saved, field by
    /// field, and is never kept; the settings in force say which did.
    #[test]
    fn the_run_s_flags_replace_what_is_saved_and_say_so() {
        let saved = HostSettings {
            names: vec!["studio.local".into()],
            port: Some(7500),
            ..on("192.168.1.20")
        };
        let plain = effective(&saved, &Overrides::default()).unwrap();
        assert!(plain.allow_other_devices);
        assert_eq!(plain.address, "192.168.1.20");
        assert_eq!(
            plain.port,
            DevicePort {
                port: 7500,
                from: PortFrom::Saved
            }
        );
        assert_eq!(plain.names, ["studio.local"]);
        assert!(plain.overridden.is_empty());

        let flags = Overrides {
            address: Some("100.64.0.1".parse().unwrap()),
            port: Some(7600),
            names: Some(vec!["mac.ts.net".into()]),
            tls: Some(("/c.pem".into(), "/k.pem".into())),
            ..Default::default()
        };
        let run = effective(&HostSettings::default(), &flags).unwrap();
        assert!(run.allow_other_devices, "--public-bind turns it on");
        assert_eq!(run.address, "100.64.0.1");
        assert_eq!(
            run.port,
            DevicePort {
                port: 7600,
                from: PortFrom::Flag
            }
        );
        assert_eq!(run.names, ["mac.ts.net"]);
        assert_eq!(
            run.tls_cert.as_deref(),
            Some(std::path::Path::new("/c.pem"))
        );
        assert_eq!(
            run.overridden,
            [
                "--public-bind",
                "--device-port",
                "--allow-host",
                "--tls-cert"
            ]
        );

        let env = Overrides {
            port_env: Some("7700".into()),
            ..Default::default()
        };
        let by_env = effective(&saved, &env).unwrap();
        assert_eq!(
            by_env.port,
            DevicePort {
                port: 7700,
                from: PortFrom::Env
            }
        );
        assert_eq!(by_env.overridden, ["OXPLOW_DEVICE_PORT"]);
        let any = Overrides {
            port: Some(0),
            ..Default::default()
        };
        assert_eq!(
            effective(&saved, &any).unwrap().port.port,
            0,
            "a free port, a test's"
        );
        let bad = Overrides {
            port_env: Some("seventy".into()),
            ..Default::default()
        };
        assert_eq!(
            effective(&saved, &bad),
            Err(SettingsError::BadPort("seventy".into()))
        );
        // A name a flag gives is checked as a saved one is.
        let public = Overrides {
            names: Some(vec!["mac.example.com".into()]),
            ..Default::default()
        };
        assert!(matches!(
            effective(&saved, &public),
            Err(SettingsError::PublicName(_))
        ));
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
}
