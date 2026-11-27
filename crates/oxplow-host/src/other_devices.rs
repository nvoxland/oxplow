//! "Allow other devices" in the host: the saved settings (`host.db`), what
//! this run's flags put over them, and the settings in force
//! (`oxplow_host_access::settings`).

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_host_access::settings::{effective, Effective, HostSettings, Overrides, SettingsError};
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::db::HostDb;

/// The settings of one host: what is saved and what this run was told.
pub struct OtherDevices {
    db: Arc<HostDb>,
    overrides: Overrides,
}

/// The settings as the Devices page reads them (`GET /host/settings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    pub saved: HostSettings,
    /// What is in force now; none when it cannot be (`problem` says why).
    pub effective: Option<Effective>,
    pub problem: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Invalid(#[from] SettingsError),
    /// The listener could not be made so (a port in use, a certificate that
    /// cannot be read): nothing was kept.
    #[error("{0}")]
    Unapplied(String),
    #[error("the settings store failed: {0}")]
    Store(#[from] rusqlite::Error),
}

impl OtherDevices {
    pub fn new(db: Arc<HostDb>, overrides: Overrides) -> Self {
        Self { db, overrides }
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// What is saved; none saved is off.
    pub fn saved(&self) -> HostSettings {
        let conn = self.db.conn();
        let read = conn
            .query_row(
                "SELECT allow, address, port, names, tls_cert, tls_key FROM other_devices WHERE id = 1",
                [],
                |r| {
                    let names: String = r.get(3)?;
                    Ok(HostSettings {
                        allow_other_devices: r.get(0)?,
                        address: r.get(1)?,
                        port: r.get(2)?,
                        names: serde_json::from_str(&names).unwrap_or_default(),
                        tls_cert: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
                        tls_key: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
                    })
                },
            )
            .optional();
        match read {
            Ok(saved) => saved.unwrap_or_default(),
            Err(e) => {
                tracing::error!(error = %e, "cannot read the settings for other devices");
                HostSettings::default()
            }
        }
    }

    /// Check `settings`, make the listener for other devices so (`server`,
    /// once `serve` has one), and keep them. What cannot work is refused and
    /// nothing changes. Answers what was kept.
    pub async fn apply(
        &self,
        settings: HostSettings,
        server: Option<&crate::public::PublicServer>,
    ) -> Result<HostSettings, SaveError> {
        let wanted = settings.checked()?;
        if let Some(server) = server {
            let in_force = effective(&wanted, &self.overrides)?;
            server
                .apply(&in_force)
                .await
                .map_err(SaveError::Unapplied)?;
        }
        self.save(wanted)
    }

    /// Check `settings` and keep them; what cannot work is refused and
    /// nothing is kept. Answers what was kept.
    pub fn save(&self, settings: HostSettings) -> Result<HostSettings, SaveError> {
        let kept = settings.checked()?;
        let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.to_string_lossy().into_owned());
        self.db.conn().execute(
            "INSERT INTO other_devices (id, allow, address, port, names, tls_cert, tls_key)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET allow = ?1, address = ?2, port = ?3,
               names = ?4, tls_cert = ?5, tls_key = ?6",
            rusqlite::params![
                kept.allow_other_devices,
                kept.address,
                kept.port,
                serde_json::to_string(&kept.names).unwrap_or_else(|_| "[]".into()),
                path(&kept.tls_cert),
                path(&kept.tls_key),
            ],
        )?;
        Ok(kept)
    }

    /// The settings in force: what is saved, under this run's flags.
    pub fn effective(&self) -> Result<Effective, SettingsError> {
        effective(&self.saved(), &self.overrides)
    }

    pub fn view(&self) -> SettingsView {
        let (effective, problem) = match self.effective() {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(e.to_string())),
        };
        SettingsView {
            saved: self.saved(),
            effective,
            problem,
        }
    }
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

    #[test]
    fn the_settings_are_kept_or_refused_and_none_saved_is_off() {
        let home = tempfile::tempdir().unwrap();
        let db = HostDb::open(home.path()).unwrap();
        let other = OtherDevices::new(db.clone(), Overrides::default());
        assert_eq!(other.saved(), HostSettings::default());

        let wanted = HostSettings {
            names: vec![" studio.local ".into()],
            port: Some(7401),
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
            ..on("192.168.1.20")
        };
        let kept = other.save(wanted).unwrap();
        assert_eq!(kept.names, ["studio.local"], "tidied");
        assert_eq!(other.saved(), kept);
        assert!(matches!(
            other.save(on("")),
            Err(SaveError::Invalid(SettingsError::NoAddress))
        ));
        assert_eq!(other.saved(), kept, "nothing kept");

        drop(other);
        drop(db);
        let again = OtherDevices::new(HostDb::open(home.path()).unwrap(), Overrides::default());
        assert_eq!(again.saved(), kept, "across a restart");
    }

    #[test]
    fn this_run_s_flags_are_in_force_and_never_kept() {
        let home = tempfile::tempdir().unwrap();
        let flags = Overrides {
            address: Some("100.64.0.1".parse().unwrap()),
            ..Default::default()
        };
        let other = OtherDevices::new(HostDb::open(home.path()).unwrap(), flags);
        other.save(HostSettings::default()).unwrap();
        let view = other.view();
        assert!(!view.saved.allow_other_devices);
        let now = view.effective.unwrap();
        assert!(now.allow_other_devices);
        assert_eq!(now.address, "100.64.0.1");
        assert_eq!(now.overridden, ["--public-bind"]);
        assert_eq!(view.problem, None);
        assert_eq!(
            other.saved(),
            HostSettings::default(),
            "the flag is not kept"
        );

        let elsewhere = tempfile::tempdir().unwrap();
        let bad = OtherDevices::new(
            HostDb::open(elsewhere.path()).unwrap(),
            Overrides {
                port_env: Some("nope".into()),
                ..Default::default()
            },
        );
        let view = bad.view();
        assert!(view.effective.is_none());
        assert!(view.problem.unwrap().contains("nope"));
    }
}
