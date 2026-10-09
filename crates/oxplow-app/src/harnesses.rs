//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` (`oxplow_harnesses`) registered under the key its
//! declaration gives — the key an agent session's `harness` names. Many are
//! registered at once, the way collectors are.

use oxplow_domain::agent::registry::{AcpAdapterRegistry, HarnessRegistry};

use crate::capabilities::{Implementation, Source};

/// Register the harnesses `declared` names (the project's extensions'
/// `implementations:`), in declaration order, and nothing else.
pub fn register_built_ins(registry: &HarnessRegistry, declared: &[Implementation]) {
    registry.set_declared(
        declared
            .iter()
            .filter(|i| i.capability == "agent_harness")
            .filter_map(|i| match i.source {
                Source::BuiltIn(entry) => oxplow_harnesses::built_in(entry, &i.id, &i.title),
                _ => None,
            })
            .collect(),
    );
}

/// One registered harness, as the session picker and Settings see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessListing {
    /// Its key (what `agents:` and a session's `harness` name).
    pub id: String,
    pub title: String,
    /// It runs an ACP agent in a chat (a structured transcript), so a
    /// session of it names one.
    pub chat: bool,
    /// The project enables it (`agents:` names it, or names none).
    pub enabled: bool,
    /// The settings it reads from its `agentConfig` entry.
    pub settings: Vec<HarnessSettingListing>,
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
pub fn listing(registry: &HarnessRegistry, agents: &[String]) -> Vec<HarnessListing> {
    use oxplow_domain::agent::harness::Transcript;
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
            chat: h.interact().transcript == Transcript::Structured,
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
        })
        .collect()
}

/// Set the ACP adapters `declared` names, in declaration order. One whose
/// config doesn't hold is left out (logged); the loader checked it against
/// the built-in's schema already.
pub fn register_acp_adapters(registry: &AcpAdapterRegistry, declared: &[Implementation]) {
    let adapters = declared
        .iter()
        .filter(|i| i.capability == "acp_adapter")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => {
                match oxplow_harnesses::acp_adapter(entry, &i.id, &i.title, &i.config)? {
                    Ok(adapter) => Some(adapter),
                    Err(error) => {
                        tracing::warn!(adapter = %i.id, %error, "an ACP adapter's config doesn't hold");
                        None
                    }
                }
            }
            _ => None,
        })
        .collect();
    registry.set(adapters);
}

#[cfg(test)]
mod tests {
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

    /// What's declared is registered under its key; what no longer is goes.
    #[test]
    fn the_declared_harnesses_are_registered() {
        let r = HarnessRegistry::new(std::sync::Arc::new(|| "claude".into()));
        register_built_ins(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "acp", "oxplow:acp"),
                declared("work_items", "oxplow", "oxplow:tasks"),
            ],
        );
        assert_eq!(r.names(), ["claude", "acp"]);
        assert_eq!(r.default().unwrap().id(), "claude");
        register_built_ins(&r, &[declared("agent_harness", "acp", "oxplow:acp")]);
        assert_eq!(r.names(), ["acp"]);
    }

    /// The declared ACP adapters are set in declaration order, each from
    /// its config; one whose config doesn't hold is left out.
    #[test]
    fn the_declared_acp_adapters_are_set_from_their_config() {
        let adapter = |id: &str, config: serde_json::Value| Implementation {
            config,
            ..declared("acp_adapter", id, "oxplow:acp-adapter")
        };
        let r = AcpAdapterRegistry::default();
        register_acp_adapters(
            &r,
            &[
                adapter(
                    "gemini",
                    serde_json::json!({ "command": "gemini", "args": ["--acp"] }),
                ),
                adapter("broken", serde_json::json!({ "args": [] })),
                declared("agent_harness", "claude", "oxplow:claude-code"),
                adapter(
                    "claude",
                    serde_json::json!({ "command": "claude-agent-acp" }),
                ),
            ],
        );
        let ids: Vec<(String, String)> = r.all().into_iter().map(|a| (a.id, a.command)).collect();
        assert_eq!(
            ids,
            [
                ("gemini".to_string(), "gemini".to_string()),
                ("claude".to_string(), "claude-agent-acp".to_string())
            ]
        );
    }

    /// The listing is the registry's, the enabled ones first in `agents:`
    /// order; with no `agents:` every harness is enabled, as declared.
    #[test]
    fn the_listing_says_which_run_acp_agents_and_which_are_enabled() {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        register_built_ins(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "acp", "oxplow:acp"),
            ],
        );
        let rows = |agents: &[&str]| {
            let agents: Vec<String> = agents.iter().map(|a| a.to_string()).collect();
            listing(&r, &agents)
                .into_iter()
                .map(|h| (h.id, h.chat, h.enabled))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&[]),
            [("claude".into(), false, true), ("acp".into(), true, true)]
        );
        assert_eq!(
            rows(&["acp"]),
            [("acp".into(), true, true), ("claude".into(), false, false)]
        );
    }

    /// A harness's settings ride its listing: opencode's model, nothing
    /// for Claude.
    #[test]
    fn the_listing_carries_each_harnesss_settings() {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        register_built_ins(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "opencode", "oxplow:opencode"),
            ],
        );
        let rows = listing(&r, &[]);
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
        assert!(keys("claude").is_empty());
    }
}
