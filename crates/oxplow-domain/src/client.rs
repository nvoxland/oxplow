//! Who is asking: the person's **device** (the Mac app, or a paired browser
//! on one host address) and the **connection** (one page load of one
//! project's app). A request carries them as its [`Caller`]; per-device
//! state is keyed by the device, page runs and the client host by the
//! connection. Both ids are made by the client (`dev-<uuid>`, `con-<uuid>`),
//! so they are checked here before they reach a row or a log.

use serde::{Deserialize, Serialize};

/// What went wrong with an id a client sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected `{prefix}<uuid>`, got `{got}`")]
pub struct BadClientId {
    pub prefix: &'static str,
    pub got: String,
}

fn check(prefix: &'static str, raw: &str) -> Result<(), BadClientId> {
    let ok = raw
        .strip_prefix(prefix)
        .and_then(|rest| uuid::Uuid::try_parse(rest).ok())
        .is_some_and(|u| u.hyphenated().to_string() == raw[prefix.len()..]);
    if ok {
        Ok(())
    } else {
        Err(BadClientId {
            prefix,
            got: raw.chars().take(80).collect(),
        })
    }
}

macro_rules! client_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A fresh id.
            pub fn generate() -> Self {
                Self(format!("{}{}", $prefix, uuid::Uuid::new_v4().hyphenated()))
            }

            pub fn parse(raw: &str) -> Result<Self, BadClientId> {
                check($prefix, raw)?;
                Ok(Self(raw.to_string()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = BadClientId;
            fn try_from(raw: String) -> Result<Self, BadClientId> {
                check($prefix, &raw)?;
                Ok(Self(raw))
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

client_id!(
    /// The person's device: the Mac app (all its windows), or a paired browser
    /// on one host address. Per-device state is keyed by it.
    DeviceId,
    "dev-"
);

client_id!(
    /// One page load of one project's app, across its socket's reconnects.
    /// Page runs, tab reports and the client host are keyed by it.
    ConnectionId,
    "con-"
);

/// The form a device takes; it picks the default shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Desktop,
    Tablet,
    Phone,
}

/// The longest name a device keeps, in characters.
pub const DEVICE_NAME_MAX: usize = 64;

/// The name a device gave, as shown to people and agents: control
/// characters dropped, trimmed, cut to [`DEVICE_NAME_MAX`], and
/// "Unnamed device" when nothing is left.
pub fn device_name(given: Option<&str>) -> String {
    let clean: String = given
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name: String = clean.trim().chars().take(DEVICE_NAME_MAX).collect();
    let name = name.trim_end();
    if name.is_empty() {
        "Unnamed device".into()
    } else {
        name.into()
    }
}

/// A device as it names itself (until the host's registry holds this).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeviceDescriptor {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
}

/// Who a request comes from. A request with none (an agent over MCP, a
/// script with the token, a test) reads per-device state as its defaults
/// and may not write it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Caller {
    pub device: DeviceId,
    pub connection: ConnectionId,
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_device_name_is_trimmed_cut_and_never_empty() {
        assert_eq!(device_name(Some("  Tab S9 \n")), "Tab S9");
        assert_eq!(device_name(Some("Tab\u{7}S9\r\n2")), "TabS92");
        assert_eq!(
            device_name(Some(&"x".repeat(500))).chars().count(),
            DEVICE_NAME_MAX
        );
        assert_eq!(device_name(Some(" \t ")), "Unnamed device");
        assert_eq!(device_name(None), "Unnamed device");
    }

    use super::*;

    #[test]
    fn an_id_is_its_prefix_and_a_uuid() {
        let device = DeviceId::generate();
        assert!(device.as_str().starts_with("dev-"));
        assert_eq!(DeviceId::parse(device.as_str()), Ok(device.clone()));
        let connection = ConnectionId::generate();
        assert!(connection.as_str().starts_with("con-"));
        assert_eq!(ConnectionId::parse(connection.as_str()), Ok(connection));
    }

    #[test]
    fn an_id_from_a_client_is_checked() {
        for bad in [
            "",
            "dev-",
            "dev-not-a-uuid",
            "con-1b4e28ba-2fa1-11d2-883f-0016d3cca427",
            "dev-1B4E28BA-2FA1-11D2-883F-0016D3CCA427",
            "dev-1b4e28ba2fa111d2883f0016d3cca427",
            " dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427",
        ] {
            assert!(DeviceId::parse(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(DeviceId::parse("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427").is_ok());
        // From JSON too: a row or an argument never holds an unchecked id.
        assert!(serde_json::from_str::<ConnectionId>("\"con-x\"").is_err());
        let ok: ConnectionId =
            serde_json::from_str("\"con-1b4e28ba-2fa1-11d2-883f-0016d3cca427\"").unwrap();
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            "\"con-1b4e28ba-2fa1-11d2-883f-0016d3cca427\""
        );
    }

    #[test]
    fn a_kind_is_one_of_three_forms() {
        assert_eq!(
            serde_json::to_string(&DeviceKind::Tablet).unwrap(),
            "\"tablet\""
        );
        assert!(serde_json::from_str::<DeviceKind>("\"browser\"").is_err());
    }
}
