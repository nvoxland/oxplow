//! Capabilities: the pieces of oxplow a project chooses an implementation
//! of (`.context/work-tracking.md` "Swappable pieces"). Core declares each
//! one here — whether it may be "none", its default implementation and
//! the features an implementation may declare — and nothing else names a
//! capability's rules.
//!
//! An implementation is declared elsewhere (a built-in an extension names,
//! a provider instance); which one is active is resolved from the
//! person's and the project's choices (`oxplow_app::capabilities`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why a capability's active implementation is the one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChosenBy {
    /// The person's own layer.
    Personal,
    /// The project's `activeProviders`.
    Project,
    /// Nothing chose: the capability's default.
    Default,
    /// The choice isn't available (its extension is disabled, its
    /// instance stopped, its id unknown).
    Fallback,
}

impl ChosenBy {
    pub fn as_str(self) -> &'static str {
        match self {
            ChosenBy::Personal => "personal",
            ChosenBy::Project => "project",
            ChosenBy::Default => "default",
            ChosenBy::Fallback => "fallback",
        }
    }
}

/// One capability, as core declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitySpec {
    /// `work_items`, `effort_policy`, …: the key under `activeProviders`.
    pub id: &'static str,
    /// How a person names it.
    pub title: &'static str,
    /// Whether a project chooses its implementation. One that can't has
    /// a single implementation, always active (`vcs`, `knowledge`).
    pub choosable: bool,
    /// Whether it may be [`NONE`]: nothing implements it, and what
    /// needs it says so.
    pub optional: bool,
    /// Whether many implementations serve it at once (every agent
    /// harness, every AI provider) rather than one chosen: each one
    /// declared is active, nothing is chosen or switched, and a need on
    /// one of its features is met by any of them. Never choosable or
    /// optional.
    pub many: bool,
    /// The implementation used when none is chosen — and, for a required
    /// capability, when the chosen one isn't available. Core always has
    /// it. For a many-capability: the one used when nothing names one.
    pub default: &'static str,
    /// The features an implementation may declare.
    pub features: &'static [&'static str],
}

/// The implementation id of "nothing implements it", for an optional
/// capability.
pub const NONE: &str = "none";

/// Every capability core declares.
pub const CAPABILITIES: &[CapabilitySpec] = &[
    CapabilitySpec {
        id: "work_items",
        title: "Work list",
        choosable: true,
        optional: true,
        many: false,
        default: "oxplow",
        features: &[
            "hierarchy",
            "comments",
            "links",
            "delete",
            "idempotent_writes",
            "ordering",
            "lists",
        ],
    },
    CapabilitySpec {
        id: "effort_policy",
        title: "Effort policy",
        choosable: true,
        optional: true,
        many: false,
        default: "oxplow",
        features: &[],
    },
    CapabilitySpec {
        id: "snapshots",
        title: "Snapshots",
        choosable: true,
        optional: false,
        many: false,
        default: "oxplow",
        features: &["contents"],
    },
    CapabilitySpec {
        id: "vcs",
        title: "Version control",
        choosable: false,
        optional: false,
        many: false,
        default: "git",
        features: &[],
    },
    CapabilitySpec {
        id: "knowledge",
        title: "Knowledge",
        choosable: false,
        optional: false,
        many: false,
        default: "oxplow",
        features: &[],
    },
    CapabilitySpec {
        id: "agent_harness",
        title: "Agent harness",
        choosable: false,
        optional: false,
        many: true,
        default: "claude",
        features: &[
            "terminal",
            "structured_transcript",
            "permission_prompts",
            "resume",
            "programmatic",
        ],
    },
    CapabilitySpec {
        id: "acp_adapter",
        title: "ACP agent",
        choosable: false,
        optional: false,
        many: true,
        default: "claude",
        features: &[],
    },
    CapabilitySpec {
        id: "ai_provider",
        title: "AI provider",
        choosable: false,
        optional: false,
        many: true,
        default: "anthropic",
        features: &["decide_native"],
    },
];

/// The capability `id`, if core declares it.
pub fn spec(id: &str) -> Option<&'static CapabilitySpec> {
    CAPABILITIES.iter().find(|c| c.id == id)
}

/// The capabilities a project chooses an implementation of.
pub fn choosable() -> impl Iterator<Item = &'static CapabilitySpec> {
    CAPABILITIES.iter().filter(|c| c.choosable)
}

/// Check a declaration's `config` against its built-in's JSON Schema
/// (`schema`, its text). The error says what's wrong, where.
pub fn check_config(schema: &str, config: &serde_json::Value) -> Result<(), String> {
    let schema: serde_json::Value =
        serde_json::from_str(schema).map_err(|e| format!("the built-in's config schema: {e}"))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| format!("the built-in's config schema: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(config)
        .map(|e| {
            let at = e.instance_path().to_string();
            if at.is_empty() {
                e.to_string()
            } else {
                format!("{at}: {e}")
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("config: {}", errors.join("; ")))
    }
}

/// Check one declared need: a host capability (`sql.read`,
/// [`crate::host_capability`]), a capability (`work_items`), or one of its
/// features (`snapshots.contents`), as core declares them.
pub fn check_need(need: &str) -> Result<(), String> {
    if crate::host_capability::host_capability(need).is_some() {
        return Ok(());
    }
    let (id, feature) = match need.split_once('.') {
        Some((id, feature)) => (id, Some(feature)),
        None => (need, None),
    };
    let Some(spec) = spec(id) else {
        return Err(format!(
            "needs `{need}`: `{id}` isn't a capability ({})",
            CAPABILITIES
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if let Some(f) = feature {
        if !spec.features.contains(&f) {
            return Err(format!(
                "needs `{need}`: `{f}` isn't a feature of `{id}` ({})",
                if spec.features.is_empty() {
                    "it has none".to_string()
                } else {
                    spec.features.join(", ")
                }
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_need_may_name_a_host_capability() {
        assert!(check_need("sql.read").is_ok());
        assert!(check_need("sql.write").is_err());
    }

    #[test]
    fn every_capability_is_declared_once_with_a_default() {
        let mut ids: Vec<&str> = CAPABILITIES.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CAPABILITIES.len());
        for c in CAPABILITIES {
            assert!(!c.default.is_empty() && c.default != NONE, "{}", c.id);
            assert!(
                c.choosable || !c.optional,
                "{}: only a choice may be none",
                c.id
            );
        }
        assert_eq!(
            choosable().map(|c| c.id).collect::<Vec<_>>(),
            ["work_items", "effort_policy", "snapshots"]
        );
        for c in CAPABILITIES.iter().filter(|c| c.many) {
            assert!(
                !c.choosable && !c.optional,
                "{}: many implementations are registered at once, never chosen or none",
                c.id
            );
        }
        assert_eq!(
            CAPABILITIES
                .iter()
                .filter(|c| c.many)
                .map(|c| (c.id, c.default))
                .collect::<Vec<_>>(),
            [
                ("agent_harness", "claude"),
                ("acp_adapter", "claude"),
                ("ai_provider", "anthropic")
            ]
        );
        assert_eq!(spec("snapshots").map(|c| c.optional), Some(false));
        assert!(spec("nope").is_none());
    }

    #[test]
    fn a_need_names_a_capability_or_one_of_its_features() {
        assert!(check_need("work_items").is_ok());
        assert!(check_need("snapshots.contents").is_ok());
        assert!(check_need("teleport")
            .unwrap_err()
            .contains("isn't a capability"));
        assert!(check_need("work_items.flying")
            .unwrap_err()
            .contains("isn't a feature"));
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    /// A declaration's `config:` is checked against its built-in's schema:
    /// the error names what's wrong.
    #[test]
    fn config_is_checked_against_a_schema() {
        let schema = r#"{"type":"object","required":["command"],"properties":{"command":{"type":"string"}},"additionalProperties":false}"#;
        assert!(check_config(schema, &serde_json::json!({"command": "gemini"})).is_ok());
        let err = check_config(schema, &serde_json::json!({})).unwrap_err();
        assert!(err.contains("command"), "{err}");
        let err =
            check_config(schema, &serde_json::json!({"command": "x", "nope": 1})).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}
