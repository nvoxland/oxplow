//! The registered agent harnesses, by key.

use std::collections::BTreeMap;
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

/// The harnesses the project's extensions declare, many at once. Cloning
/// shares them.
#[derive(Clone)]
pub struct HarnessRegistry {
    harnesses: Arc<RwLock<BTreeMap<String, Arc<dyn AgentHarness>>>>,
    /// The one a new session runs when it names none.
    default: ActiveSource,
}

impl HarnessRegistry {
    /// A registry whose default harness is what `default` says each time
    /// it's asked.
    pub fn new(default: ActiveSource) -> Self {
        Self {
            harnesses: Arc::default(),
            default,
        }
    }

    pub fn register(&self, harness: Arc<dyn AgentHarness>) {
        self.harnesses
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(harness.id().to_string(), harness);
    }

    pub fn unregister(&self, id: &str) {
        self.harnesses
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// Every registered key, sorted.
    pub fn names(&self) -> Vec<String> {
        self.harnesses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Every registered harness, by key.
    pub fn all(&self) -> Vec<Arc<dyn AgentHarness>> {
        self.harnesses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn AgentHarness>, UnknownHarness> {
        self.harnesses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| UnknownHarness {
                harness: id.to_string(),
                registered: self.names(),
            })
    }

    /// The harness a session runs when it names none.
    pub fn default(&self) -> Result<Arc<dyn AgentHarness>, UnknownHarness> {
        self.get(&(self.default)())
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
    use crate::agent::harness::{
        Gate, HarnessError, Input, Interact, Launch, LaunchInput, Transcript,
    };

    struct Fake(&'static str);

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
                input: Input::Keystrokes,
                gate: Gate::Harness,
            }
        }
        fn launch(&self, _: &LaunchInput<'_>) -> Result<Launch, HarnessError> {
            Err(HarnessError::Config("a fake".into()))
        }
        fn instruction_files(&self) -> &[&str] {
            &[]
        }
        fn env_markers(&self) -> &[&str] {
            &[]
        }
        fn refresh_text(
            &self,
            _: &std::path::Path,
            _: &crate::agent::text::AgentText,
        ) -> Result<(), HarnessError> {
            Ok(())
        }
        fn writing_tools(&self) -> &[&str] {
            &[]
        }
        fn turns(&self, _: &str) -> Vec<crate::agent::observe::Turn> {
            Vec::new()
        }
        fn token_readings(
            &self,
            _: &crate::agent::observe::OtlpRecord<'_>,
        ) -> Vec<crate::agent::observe::TokenReading> {
            Vec::new()
        }
        fn render(&self, _: &crate::agent::observe::HookAnswer) -> serde_json::Value {
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
}
