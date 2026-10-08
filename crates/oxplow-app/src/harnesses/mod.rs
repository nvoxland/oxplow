//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` registered under the key its declaration gives — the key
//! an agent session's `harness` names. Many are registered at once, the way
//! collectors are.

mod acp;
mod claude;
mod codex;
mod opencode;

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::HarnessRegistry;

use crate::capabilities::{Implementation, Source};

/// The harness a built-in `entry` is, registered under `id`.
pub fn built_in(entry: &str, id: &str, title: &str) -> Option<Arc<dyn AgentHarness>> {
    let named = Named {
        id: id.into(),
        title: title.into(),
    };
    Some(match entry {
        "oxplow:claude-code" => Arc::new(claude::Claude(named)),
        "oxplow:codex-cli" => Arc::new(codex::Codex(named)),
        "oxplow:opencode" => Arc::new(opencode::Opencode(named)),
        "oxplow:acp" => Arc::new(acp::Acp(named)),
        _ => return None,
    })
}

/// Register the harnesses `declared` names (the project's extensions'
/// `implementations:`), and unregister the ones it no longer does.
pub fn register_built_ins(registry: &HarnessRegistry, declared: &[Implementation]) {
    let harnesses: Vec<Arc<dyn AgentHarness>> = declared
        .iter()
        .filter(|i| i.capability == "agent_harness")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => built_in(entry, &i.id, &i.title),
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

/// A harness's key and title, as its declaration gives them.
struct Named {
    id: String,
    title: String,
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
