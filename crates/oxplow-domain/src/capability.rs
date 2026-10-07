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
    /// The implementation used when none is chosen — and, for a required
    /// capability, when the chosen one isn't available. Core always has
    /// it.
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
        default: "oxplow",
        features: &[
            "hierarchy",
            "comments",
            "links",
            "delete",
            "idempotent_writes",
        ],
    },
    CapabilitySpec {
        id: "effort_policy",
        title: "Effort policy",
        choosable: true,
        optional: true,
        default: "oxplow",
        features: &[],
    },
    CapabilitySpec {
        id: "snapshots",
        title: "Snapshots",
        choosable: true,
        optional: false,
        default: "oxplow",
        features: &["contents"],
    },
    CapabilitySpec {
        id: "vcs",
        title: "Version control",
        choosable: false,
        optional: false,
        default: "git",
        features: &[],
    },
    CapabilitySpec {
        id: "knowledge",
        title: "Knowledge",
        choosable: false,
        optional: false,
        default: "oxplow",
        features: &[],
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

/// Check one declared need: a capability (`work_items`), or one of its
/// features (`snapshots.contents`), as core declares them.
pub fn check_need(need: &str) -> Result<(), String> {
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
