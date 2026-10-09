//! The registered agent harnesses, by key.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use super::acp_adapter::AcpAdapter;
use super::harness::AgentHarness;
use crate::work_items::ActiveSource;

/// A harness key nothing registers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no agent harness `{harness}` (registered: {})", registered.join(", "))]
pub struct UnknownHarness {
    pub harness: String,
    pub registered: Vec<String>,
}

/// The harnesses the project's extensions declare, many at once, in
/// declaration order, and the provider instances registered on their own.
/// A catalog reload restates the declared ones ([`Self::set_declared`])
/// and never touches an instance ([`Self::register`]). Cloning shares
/// them.
#[derive(Clone)]
pub struct HarnessRegistry {
    harnesses: Arc<RwLock<Vec<Arc<dyn AgentHarness>>>>,
    /// The keys `set_declared` put there.
    declared: Arc<RwLock<BTreeSet<String>>>,
    /// The key of the one a new session runs when it names none; empty for
    /// the first registered.
    default: ActiveSource,
}

impl HarnessRegistry {
    /// A registry whose default harness is what `default` says each time
    /// it's asked (empty: the first registered).
    pub fn new(default: ActiveSource) -> Self {
        Self {
            harnesses: Arc::default(),
            declared: Arc::default(),
            default,
        }
    }

    /// The declared harnesses are now `harnesses`, in this order, ahead of
    /// the registered instances: those declared before and not now go,
    /// and an instance stays.
    pub fn set_declared(&self, harnesses: Vec<Arc<dyn AgentHarness>>) {
        let mut all = self.harnesses.write().unwrap_or_else(|e| e.into_inner());
        let mut declared = self.declared.write().unwrap_or_else(|e| e.into_inner());
        let instances: Vec<_> = all
            .drain(..)
            .filter(|h| !declared.contains(h.id()))
            .filter(|h| !harnesses.iter().any(|d| d.id() == h.id()))
            .collect();
        *declared = harnesses.iter().map(|h| h.id().to_string()).collect();
        *all = harnesses;
        all.extend(instances);
    }

    /// Register `harness` (a provider's instance), replacing one with its
    /// key in place, else last. A reload keeps it.
    pub fn register(&self, harness: Arc<dyn AgentHarness>) {
        let mut harnesses = self.harnesses.write().unwrap_or_else(|e| e.into_inner());
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(harness.id());
        match harnesses.iter_mut().find(|h| h.id() == harness.id()) {
            Some(slot) => *slot = harness,
            None => harnesses.push(harness),
        }
    }

    pub fn unregister(&self, id: &str) {
        self.harnesses
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|h| h.id() != id);
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// Whether a harness has the key `id`.
    pub fn has(&self, id: &str) -> bool {
        self.all().iter().any(|h| h.id() == id)
    }

    /// Every registered key, in declaration order.
    pub fn names(&self) -> Vec<String> {
        self.all().iter().map(|h| h.id().to_string()).collect()
    }

    /// Every registered harness, in declaration order.
    pub fn all(&self) -> Vec<Arc<dyn AgentHarness>> {
        self.harnesses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn AgentHarness>, UnknownHarness> {
        let all = self.all();
        all.iter()
            .find(|h| h.id() == id)
            .cloned()
            .ok_or_else(|| UnknownHarness {
                harness: id.to_string(),
                registered: all.iter().map(|h| h.id().to_string()).collect(),
            })
    }

    /// The harness a session runs when it names none: the project's choice,
    /// else the first registered.
    pub fn default(&self) -> Result<Arc<dyn AgentHarness>, UnknownHarness> {
        let named = (self.default)();
        if named.is_empty() {
            return self.all().into_iter().next().ok_or_else(|| UnknownHarness {
                harness: String::new(),
                registered: Vec::new(),
            });
        }
        self.get(&named)
    }
}

/// The ACP adapters the project's extensions declare, in declaration
/// order: the agents an `acp` session can run before the project's own
/// `acpAgents:`. Cloning shares them.
#[derive(Clone, Default)]
pub struct AcpAdapterRegistry {
    adapters: Arc<RwLock<Vec<AcpAdapter>>>,
}

impl AcpAdapterRegistry {
    /// Replace what's declared.
    pub fn set(&self, adapters: Vec<AcpAdapter>) {
        *self.adapters.write().unwrap_or_else(|e| e.into_inner()) = adapters;
    }

    /// Every declared adapter, in declaration order.
    pub fn all(&self) -> Vec<AcpAdapter> {
        self.adapters
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::harness::{HarnessError, Interact, Launch, LaunchInput, Transcript};

    struct Fake(&'static str);

    #[async_trait::async_trait]
    impl AgentHarness for Fake {
        fn id(&self) -> &str {
            self.0
        }
        fn title(&self) -> &str {
            self.0
        }
        fn interact(&self) -> Interact {
            Interact {
                transcript: Transcript::Terminal,
            }
        }
        async fn launch(&self, _: &LaunchInput) -> Result<Launch, HarnessError> {
            Err(HarnessError::Config("a fake".into()))
        }
        async fn tool_use(&self, _: &serde_json::Value) -> Option<crate::agent::tool::ToolUse> {
            None
        }
        async fn render(&self, _: &crate::agent::observe::HookAnswer) -> serde_json::Value {
            serde_json::Value::Null
        }
    }

    /// An unknown key names the registered ones; the default is read when
    /// asked.
    #[test]
    fn an_unknown_harness_names_the_registered() {
        let r = HarnessRegistry::new(Arc::new(|| "codex".into()));
        r.register(Arc::new(Fake("claude")));
        let err = r.get("codex").err().unwrap();
        assert_eq!(
            err,
            UnknownHarness {
                harness: "codex".into(),
                registered: vec!["claude".into()]
            }
        );
        assert!(err.to_string().contains("claude"));
        r.register(Arc::new(Fake("codex")));
        assert_eq!(r.default().unwrap().id(), "codex");
        r.unregister("codex");
        assert!(r.default().is_err());
    }

    /// Registration keeps declaration order; with no choice named, the
    /// default is the first registered.
    #[test]
    fn the_default_is_the_projects_else_the_first_declared() {
        let r = HarnessRegistry::new(Arc::new(String::new));
        assert!(r.default().is_err());
        r.set_declared(vec![Arc::new(Fake("codex")), Arc::new(Fake("claude"))]);
        assert_eq!(r.names(), ["codex", "claude"]);
        assert_eq!(r.default().unwrap().id(), "codex");
        r.register(Arc::new(Fake("acp")));
        r.register(Arc::new(Fake("codex")));
        assert_eq!(r.names(), ["codex", "claude", "acp"]);
    }

    /// A catalog reload restates the declared harnesses — one no longer
    /// declared goes — and keeps a registered instance after them.
    #[test]
    fn a_reload_restates_the_declared_and_keeps_an_instance() {
        let r = HarnessRegistry::new(Arc::new(String::new));
        r.set_declared(vec![Arc::new(Fake("claude")), Arc::new(Fake("old"))]);
        r.register(Arc::new(Fake("acme")));
        assert_eq!(r.names(), ["claude", "old", "acme"]);
        r.set_declared(vec![Arc::new(Fake("codex")), Arc::new(Fake("claude"))]);
        assert_eq!(r.names(), ["codex", "claude", "acme"]);
        r.set_declared(Vec::new());
        assert_eq!(r.names(), ["acme"]);
        assert!(r.has("acme"));
        r.unregister("acme");
        assert!(!r.has("acme"));
    }
}
