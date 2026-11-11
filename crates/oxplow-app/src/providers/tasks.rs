//! An external provider's tasks (P7.A1): [`ExternalTaskTracker`] is
//! the [`TaskVerbs`] the `task.*` commands dispatch to for an
//! enabled instance's items. Its verbs are not commands of their own —
//! `task.<verb>` is the one write surface, audited once — so each
//! call checks the input against the verb's declared `input_schema`,
//! invokes the process, and returns the events it recorded (only types it
//! declares; a `task.recorded` only for its own items).
//!
//! The verbs and their inputs are the task tracker contract every provider
//! implements — the `v_task` columns: `create { title, body?,
//! parent_uri?, stage, native? }`, `update { uri, title?, body?,
//! parent_uri? ("" detaches), stage?, native? }`, `transition { uri, to }`
//! (each stage one it declares: core resolved a bucket to it), `link { uri, target,
//! link_type }`, `comment { ref, body }` and `delete { ref }`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::tasks::{
    TaskTracker, TaskTrackerFeatures, TaskTrackers, TaskVerbs, VerbCall, VerbOutcome,
};
use oxplow_domain::{CommandCall, CommandError, InputValidator};
use serde_json::Value;

use super::endpoint::Endpoint;
use super::registry::CapabilityHost;

/// A provider's declared task tracker features.
pub fn features_of(features: &Value) -> Result<TaskTrackerFeatures, String> {
    serde_json::from_value(features.clone()).map_err(|e| e.to_string())
}

/// The task tracker's side of the host: a started instance is a task tracker
/// in `Services.tasks`, its features what the host read of them.
pub struct TaskTrackerHost(pub TaskTrackers);

impl TaskTrackerHost {
    pub const CAPABILITY: &'static str = "task_tracker";
}

impl CapabilityHost for TaskTrackerHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.0.get(id).is_ok()
    }

    fn prepare(
        &self,
        instance: &Arc<dyn Endpoint>,
    ) -> Result<super::registry::PreparedProviderCapability, String> {
        let provider = ExternalTaskTracker::provider(instance)?;
        let features = serde_json::to_value(provider.features).map_err(|e| e.to_string())?;
        let registry = self.0.clone();
        Ok(super::registry::PreparedProviderCapability::new(
            features,
            move || registry.register(provider),
        ))
    }

    fn retire(&self, id: &str) {
        self.0.unregister(id);
    }
}

pub struct ExternalTaskTracker {
    instance: Arc<dyn Endpoint>,
    /// Each declared verb's compiled input schema.
    inputs: BTreeMap<String, InputValidator>,
}

impl ExternalTaskTracker {
    /// The capability provider over a started instance: its id, its
    /// declared features, and its verbs.
    pub fn provider(instance: &Arc<dyn Endpoint>) -> Result<TaskTracker, String> {
        let decl = instance
            .declared()
            .capabilities
            .iter()
            .find(|c| c.capability == TaskTrackerHost::CAPABILITY)
            .ok_or("it doesn't declare the tasks capability")?;
        let features = features_of(&decl.features)?;
        let inputs = instance
            .declared()
            .commands
            .iter()
            .filter(|c| {
                oxplow_domain::capability::TASK_TRACKER
                    .verb(c.name.as_str())
                    .is_some()
            })
            .map(|c| {
                InputValidator::compile(&c.input_schema)
                    .map(|v| (c.name.clone(), v))
                    .map_err(|e| format!("verb `{}`: {e}", c.name))
            })
            .collect::<Result<_, _>>()?;
        Ok(TaskTracker {
            id: instance.id().to_string(),
            id_pattern: instance.id_pattern().map(str::to_string),
            sink: false,
            stages: instance.stages().to_vec(),
            features,
            verbs: Arc::new(ExternalTaskTracker {
                instance: instance.clone(),
                inputs,
            }),
        })
    }
}

#[async_trait]
impl TaskVerbs for ExternalTaskTracker {
    async fn restart(&self) {
        self.instance.restart().await;
    }

    async fn invoke(
        &self,
        call: VerbCall,
        verb: &str,
        input: Value,
    ) -> Result<VerbOutcome, CommandError> {
        let actor = call.actor().clone();
        let id = self.instance.id();
        let validator = self.inputs.get(verb).ok_or_else(|| CommandError::Invalid {
            field: None,
            message: format!("{id} tasks don't support `{verb}`"),
        })?;
        validator.check(&input).map_err(|e| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field,
                message: format!("{id} `{verb}`: {message}"),
            },
            other => other,
        })?;
        let out = self
            .instance
            .invoke(&call.run, verb, input, call.idempotency_key.clone())
            .await?;
        let events = out
            .events
            .into_iter()
            .map(|d| self.instance.envelope(&actor, d))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(VerbOutcome {
            result: out.result,
            events,
            inverse: out.inverse.map(|c| CommandCall {
                name: c.command,
                input: c.input,
            }),
        })
    }
}
