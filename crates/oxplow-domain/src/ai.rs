//! What a person may forbid an AI model binding (`restrict:` on a role's
//! `oxplow.ai.roles.<role>` assignment), and how a role names its setting.
//! An extension's profile or job chooses which binding it uses; only a
//! person decides what that binding may do, and no extension can lift a
//! restriction.

use serde::{Deserialize, Serialize};

const ROLE_KEY_PREFIX: &str = "oxplow.ai.roles.";

/// The setting key of a role's assignment: `main` is
/// `oxplow.ai.roles.main`; an extension role `namespace/profile` is
/// `oxplow.ai.roles.namespace.profile`.
pub fn role_setting_key(role: &str) -> String {
    format!("{ROLE_KEY_PREFIX}{}", role.replacen('/', ".", 1))
}

/// The role a setting key assigns, the inverse of [`role_setting_key`].
pub fn role_from_setting_key(key: &str) -> Option<String> {
    Some(key.strip_prefix(ROLE_KEY_PREFIX)?.replacen('.', "/", 1))
}

/// One thing a binding may be restricted from.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AiRestriction {
    /// Any tool: a request offering the model tools to call.
    Tools,
    /// The `query_sql` tool: reading oxplow's models through SQL.
    Sql,
    /// A streamed reply.
    Streaming,
    /// Image input.
    Images,
}

impl AiRestriction {
    pub const ALL: [AiRestriction; 4] = [
        AiRestriction::Tools,
        AiRestriction::Sql,
        AiRestriction::Streaming,
        AiRestriction::Images,
    ];

    /// Its name in configuration.
    pub fn name(self) -> &'static str {
        match self {
            AiRestriction::Tools => "tools",
            AiRestriction::Sql => "sql",
            AiRestriction::Streaming => "streaming",
            AiRestriction::Images => "images",
        }
    }

    /// What it forbids, as a person reads it after "no ".
    pub fn forbids(self) -> &'static str {
        match self {
            AiRestriction::Tools => "tools",
            AiRestriction::Sql => "SQL",
            AiRestriction::Streaming => "streaming",
            AiRestriction::Images => "image input",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names a person writes in `restrict:`.
    #[test]
    fn restrictions_are_named_as_written() {
        for r in AiRestriction::ALL {
            let written = serde_json::to_value(r).unwrap();
            assert_eq!(written, serde_json::json!(r.name()));
            assert_eq!(serde_json::from_value::<AiRestriction>(written).unwrap(), r);
        }
        assert!(serde_json::from_value::<AiRestriction>(serde_json::json!("network")).is_err());
    }
}

#[cfg(test)]
mod role_key_tests {
    use super::*;

    #[test]
    fn a_role_and_its_setting_key_round_trip() {
        for role in ["main", "acme/review"] {
            assert_eq!(
                role_from_setting_key(&role_setting_key(role)).as_deref(),
                Some(role)
            );
        }
        assert_eq!(
            role_setting_key("acme/review"),
            "oxplow.ai.roles.acme.review"
        );
        assert_eq!(role_from_setting_key("oxplow.ai.openai.enabled"), None);
    }
}
