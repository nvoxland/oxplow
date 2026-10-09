//! An external provider's work items (P7.A1): [`ExternalWorkItems`] is
//! the [`WorkItemVerbs`] the `work_item.*` commands dispatch to for an
//! enabled instance's items. Its verbs are not commands of their own —
//! `work_item.<verb>` is the one write surface, audited once — so each
//! call checks the input against the verb's declared `input_schema`,
//! invokes the process, and returns the events it recorded (only types it
//! declares; a `work_item.recorded` only for its own items).
//!
//! The verbs and their inputs are the work-items contract every provider
//! implements — the `v_work_item` columns: `create { title, body?,
//! parent_ref?, state?, native_state?, native? }`, `update { ref, title?,
//! body?, parent_ref? ("" detaches), state?, native_state?, native? }`,
//! `transition { ref, to, native_state? }`, `link { ref, target,
//! link_type }`, `comment { ref, body }` and `delete { ref }`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::work_items::{
    VerbCall, VerbOutcome, WorkItemVerbs, WorkItemsFeatures, WorkItemsProvider, WorkItemsRegistry,
};
use oxplow_domain::{CommandCall, CommandError, InputValidator};
use serde_json::Value;

use super::registry::{CapabilityHost, Instance};

/// A provider's declared work-items features.
pub fn features_of(features: &Value) -> Result<WorkItemsFeatures, String> {
    serde_json::from_value(features.clone()).map_err(|e| e.to_string())
}

/// The work list's side of the host: a started instance is a work list
/// in `Services.work_items`, its features what the host read of them.
pub struct WorkItemsHost(pub WorkItemsRegistry);

impl WorkItemsHost {
    pub const CAPABILITY: &'static str = "work_items";
}

impl CapabilityHost for WorkItemsHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.0.get(id).is_ok()
    }

    fn admit(&self, instance: &Arc<Instance>) -> Result<Value, String> {
        let provider = ExternalWorkItems::provider(instance)?;
        let features = serde_json::to_value(provider.features).map_err(|e| e.to_string())?;
        self.0.register(provider);
        Ok(features)
    }

    fn retire(&self, id: &str) {
        self.0.unregister(id);
    }
}

pub struct ExternalWorkItems {
    instance: Arc<Instance>,
    /// Each declared verb's compiled input schema.
    inputs: BTreeMap<String, InputValidator>,
}

impl ExternalWorkItems {
    /// The capability provider over a started instance: its id, its
    /// declared features, and its verbs.
    pub fn provider(instance: &Arc<Instance>) -> Result<WorkItemsProvider, String> {
        let decl = instance
            .declared
            .capabilities
            .iter()
            .find(|c| c.capability == WorkItemsHost::CAPABILITY)
            .ok_or("it doesn't declare the work_items capability")?;
        let features = features_of(&decl.features)?;
        let inputs = instance
            .declared
            .commands
            .iter()
            .filter(|c| {
                oxplow_domain::capability::WORK_ITEMS
                    .verb(c.name.as_str())
                    .is_some()
            })
            .map(|c| {
                InputValidator::compile(&c.input_schema)
                    .map(|v| (c.name.clone(), v))
                    .map_err(|e| format!("verb `{}`: {e}", c.name))
            })
            .collect::<Result<_, _>>()?;
        Ok(WorkItemsProvider {
            id: instance.id.clone(),
            id_pattern: instance.spec.id_pattern.clone(),
            sink: false,
            features,
            verbs: Arc::new(ExternalWorkItems {
                instance: instance.clone(),
                inputs,
            }),
        })
    }
}

#[async_trait]
impl WorkItemVerbs for ExternalWorkItems {
    async fn restart(&self) {
        self.instance.end_process().await;
    }

    async fn invoke(
        &self,
        call: VerbCall<'_>,
        verb: &str,
        input: Value,
    ) -> Result<VerbOutcome, CommandError> {
        let actor = call.actor;
        let id = &self.instance.id;
        // The thread a create is filed on is oxplow's record, not the
        // tracker's (tsk1058): it anchors the item, the provider never sees it.
        let mut input = input;
        let filed_on = match (verb, &mut input) {
            ("create", Value::Object(fields)) => fields
                .remove("thread")
                .and_then(|t| t.as_str().and_then(|t| t.parse().ok())),
            _ => None,
        };
        let validator = self.inputs.get(verb).ok_or_else(|| CommandError::Invalid {
            field: None,
            message: format!("{id} work items don't support `{verb}`"),
        })?;
        validator.check(&input).map_err(|e| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field,
                message: format!("{id} `{verb}`: {message}"),
            },
            other => other,
        })?;
        // Its `host/call`s name this key: they're counted with the
        // `work_item.<verb>` run.
        let key = call
            .idempotency_key
            .unwrap_or_else(|| format!("call:{}", uuid::Uuid::new_v4().simple()));
        self.instance.host_calls.begin(&key, call.trace);
        let out = self.instance.invoke(verb, input, Some(key.clone())).await;
        self.instance.host_calls.finish(&key);
        let out = out?;
        let events = out
            .events
            .into_iter()
            .map(|d| self.instance.envelope(actor, filed_on, d))
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
