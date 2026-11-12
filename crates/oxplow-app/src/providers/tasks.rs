//! The task tracker's side of the provider host: an external provider
//! program that fills the task model (`oxplow/task_record`, bundled's).
//! It has no verbs core calls — its own system takes the writes — so a
//! started instance only needs to be known as running; its records reach
//! the model through the read adapter (`providers::model_records`) and its
//! stages, fields and id pattern through the registry's provider fills
//! (`ModelRegistry::with_provider_fills`).

use std::collections::BTreeSet;
use std::sync::Arc;

use super::endpoint::Endpoint;
use super::registry::{CapabilityHost, PreparedProviderCapability};

/// The running task tracker instances.
#[derive(Default)]
pub struct TaskTrackerHost {
    running: Arc<parking_lot::Mutex<BTreeSet<String>>>,
}

impl TaskTrackerHost {
    pub const CAPABILITY: &'static str = "task_tracker";
}

impl CapabilityHost for TaskTrackerHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.running.lock().contains(id)
    }

    fn prepare(&self, instance: &Arc<dyn Endpoint>) -> Result<PreparedProviderCapability, String> {
        let decl = instance
            .declared()
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it doesn't declare the task tracker capability")?;
        let features = decl.features.clone();
        let (running, id) = (self.running.clone(), instance.id().to_string());
        Ok(PreparedProviderCapability::new(features, move || {
            running.lock().insert(id);
        }))
    }

    fn retire(&self, id: &str) {
        self.running.lock().remove(id);
    }
}
