//! What a person may forbid an AI model binding (`restrict:` on a role's
//! binding in `ai.yaml` or `project.yaml`'s `ai.roles`). An extension's
//! profile or job chooses which binding it uses; only a person decides
//! what that binding may do, and no extension can lift a restriction.

use serde::{Deserialize, Serialize};

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
