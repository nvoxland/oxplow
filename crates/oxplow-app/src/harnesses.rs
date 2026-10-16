//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` (`oxplow_harnesses`) registered under the key its
//! declaration gives — the key an agent session's `harness` names. Many are
//! registered at once, the way collectors are.

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::HarnessRegistry;

use crate::capabilities::{Implementation, Source};

/// Harnesses prepared from one declaration snapshot, without registry writes.
pub struct PreparedHarnesses {
    harnesses: Vec<Arc<dyn AgentHarness>>,
}

impl PreparedHarnesses {
    pub fn publish(self, registry: &HarnessRegistry) {
        registry.set_declared(self.harnesses);
    }
}

pub fn prepare_built_ins(declared: &[Implementation]) -> PreparedHarnesses {
    let harnesses = declared
        .iter()
        .filter(|i| i.capability == "agent_harness")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => oxplow_harnesses::built_in(entry, &i.id, &i.title),
            _ => None,
        })
        .collect();
    PreparedHarnesses { harnesses }
}

/// One registered harness, as the session picker and Settings see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessListing {
    /// Its key (what `agents:` and a session's `harness` name).
    pub id: String,
    pub title: String,
    /// The project enables it (`agents:` names it, or names none).
    pub enabled: bool,
    /// The settings it reads from its `agentConfig` entry.
    pub settings: Vec<HarnessSettingListing>,
    /// The `agent_harness` features it declares (`resume`,
    /// `conversation_list`, …): what the UI offers for its sessions.
    pub features: Vec<String>,
}

/// A harness setting, as Settings → Agents shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSettingListing {
    pub key: String,
    pub title: String,
    pub hint: String,
    pub placeholder: String,
}

/// Every registered harness in priority order: the ones `agents:` names,
/// in its order (the first is a new session's default), then the rest as
/// declared.
pub fn listing(
    registry: &HarnessRegistry,
    capabilities: &crate::capabilities::CapabilityRegistry,
    agents: &[String],
) -> Vec<HarnessListing> {
    let mut all = registry.all();
    all.sort_by_key(|h| {
        agents
            .iter()
            .position(|a| a == h.id())
            .unwrap_or(agents.len())
    });
    all.iter()
        .map(|h| HarnessListing {
            id: h.id().to_string(),
            title: h.title().to_string(),
            enabled: agents.is_empty() || agents.iter().any(|a| a == h.id()),
            settings: h
                .settings()
                .into_iter()
                .map(|s| HarnessSettingListing {
                    key: s.key,
                    title: s.title,
                    hint: s.hint,
                    placeholder: s.placeholder,
                })
                .collect(),
            features: capabilities
                .get("agent_harness", h.id())
                .and_then(|i| i.features.as_object().cloned())
                .map(|f| {
                    f.into_iter()
                        .filter(|(_, on)| on.as_bool() == Some(true))
                        .map(|(name, _)| name)
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    fn register(registry: &HarnessRegistry, declared: &[Implementation]) {
        prepare_built_ins(declared).publish(registry);
    }

    use super::*;

    fn declared(capability: &str, id: &str, entry: &'static str) -> Implementation {
        Implementation {
            capability: capability.into(),
            id: id.into(),
            title: id.into(),
            extension: Some("oxplow-foundation".into()),
            source: Source::BuiltIn(entry),
            features: serde_json::json!({}),
            fields: serde_json::json!([]),
            id_pattern: None,
            config: serde_json::json!({}),
        }
    }

    fn no_capabilities() -> crate::capabilities::CapabilityRegistry {
        crate::capabilities::CapabilityRegistry::new(
            Vec::new(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        )
    }

    /// A harness's declared features ride its listing, so the UI offers
    /// what its sessions can do (choosing another conversation).
    #[test]
    fn the_listing_carries_each_harnesss_features() {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        let claude = declared("agent_harness", "claude", "oxplow:claude-code");
        register(&r, std::slice::from_ref(&claude));
        let capabilities = no_capabilities();
        capabilities.set_declared(vec![Implementation {
            features: serde_json::json!({ "resume": true, "conversation_list": true, "transcript": false }),
            ..claude
        }]);
        let mut features = listing(&r, &capabilities, &[])[0].features.clone();
        features.sort();
        assert_eq!(features, ["conversation_list", "resume"]);
        assert!(listing(&r, &no_capabilities(), &[])[0].features.is_empty());
    }

    /// What's declared is registered under its key; what no longer is goes.
    #[test]
    fn the_declared_harnesses_are_registered() {
        let r = HarnessRegistry::new(std::sync::Arc::new(|| "claude".into()));
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "codex", "oxplow:codex-cli"),
                declared("work_items", "oxplow", "oxplow:tasks"),
            ],
        );
        assert_eq!(r.names(), ["claude", "codex"]);
        assert_eq!(r.default().unwrap().id(), "claude");
        register(
            &r,
            &[declared("agent_harness", "codex", "oxplow:codex-cli")],
        );
        assert_eq!(r.names(), ["codex"]);
    }

    /// The listing is the registry's, the enabled ones first in `agents:`
    /// order; with no `agents:` every harness is enabled, as declared.
    #[test]
    fn the_listing_says_which_are_enabled() {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "codex", "oxplow:codex-cli"),
                declared("agent_harness", "opencode", "oxplow:opencode"),
            ],
        );
        let rows = |agents: &[&str]| {
            let agents: Vec<String> = agents.iter().map(|a| a.to_string()).collect();
            listing(&r, &no_capabilities(), &agents)
                .into_iter()
                .map(|h| (h.id, h.enabled))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&[]),
            [
                ("claude".into(), true),
                ("codex".into(), true),
                ("opencode".into(), true)
            ]
        );
        assert_eq!(
            rows(&["opencode"]),
            [
                ("opencode".into(), true),
                ("claude".into(), false),
                ("codex".into(), false)
            ]
        );
    }

    /// A harness's settings ride its listing: opencode's model, Claude's MCP
    /// servers.
    #[test]
    fn the_listing_carries_each_harnesss_settings() {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "opencode", "oxplow:opencode"),
            ],
        );
        let rows = listing(&r, &no_capabilities(), &[]);
        let keys = |id: &str| -> Vec<String> {
            rows.iter()
                .find(|h| h.id == id)
                .unwrap()
                .settings
                .iter()
                .map(|s| s.key.clone())
                .collect()
        };
        assert_eq!(keys("opencode"), ["model"]);
        assert_eq!(keys("claude"), ["mcpServers", "model", "effort"]);
    }
}
