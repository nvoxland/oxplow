//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` (`oxplow_harnesses`) registered under the key its
//! declaration gives — the key an agent session's `harness` names. Many are
//! registered at once, the way collectors are.

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::{AcpAdapterRegistry, HarnessRegistry};

use crate::capabilities::{Implementation, Source};

/// Register the harnesses `declared` names (the project's extensions'
/// `implementations:`), and unregister the ones it no longer does.
pub fn register_built_ins(registry: &HarnessRegistry, declared: &[Implementation]) {
    let harnesses: Vec<Arc<dyn AgentHarness>> = declared
        .iter()
        .filter(|i| i.capability == "agent_harness")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => oxplow_harnesses::built_in(entry, &i.id, &i.title),
            _ => None,
        })
        .collect();
    for gone in registry
        .names()
        .into_iter()
        .filter(|n| !harnesses.iter().any(|h| h.id() == n))
    {
        registry.unregister(&gone);
    }
    for h in harnesses {
        registry.register(h);
    }
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
        let r = HarnessRegistry::new(Arc::new(|| "claude".into()));
        register_built_ins(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "acp", "oxplow:acp"),
                declared("work_items", "oxplow", "oxplow:tasks"),
            ],
        );
        assert_eq!(r.names(), ["acp", "claude"]);
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
}
