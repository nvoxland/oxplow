//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` (`oxplow_harnesses`) registered under the key its
//! declaration gives — the key an agent session's `harness` names. Many are
//! registered at once, the way collectors are.

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::HarnessRegistry;

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
}
