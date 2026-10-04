//! An external provider's work items (P7.A1): [`ExternalWorkItems`] is
//! the [`ExternalVerbs`] the `work_item.*` commands dispatch to for an
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
use oxplow_domain::work_items::{ExternalVerbs, VerbOutcome, WorkItemsFeatures, WorkItemsProvider};
use oxplow_domain::{Actor, CommandCall, CommandError, InputValidator};
use serde_json::Value;

use super::registry::Instance;
use super::spec;

/// A provider's declared work-items features.
pub fn features_of(features: &Value) -> Result<WorkItemsFeatures, String> {
    serde_json::from_value(features.clone()).map_err(|e| e.to_string())
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
            .find(|c| c.capability == spec::WORK_ITEMS)
            .ok_or("it doesn't declare the work_items capability")?;
        let features = features_of(&decl.features)?;
        let inputs = instance
            .declared
            .commands
            .iter()
            .filter(|c| oxplow_domain::work_items::VERBS.contains(&c.name.as_str()))
            .map(|c| {
                InputValidator::compile(&c.input_schema)
                    .map(|v| (c.name.clone(), v))
                    .map_err(|e| format!("verb `{}`: {e}", c.name))
            })
            .collect::<Result<_, _>>()?;
        Ok(WorkItemsProvider {
            id: instance.id.clone(),
            features,
            external: Some(Arc::new(ExternalWorkItems {
                instance: instance.clone(),
                inputs,
            })),
        })
    }
}

#[async_trait]
impl ExternalVerbs for ExternalWorkItems {
    async fn restart(&self) {
        self.instance.end_process().await;
    }

    async fn invoke(
        &self,
        actor: &Actor,
        verb: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<VerbOutcome, CommandError> {
        let id = &self.instance.id;
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
        let out = self.instance.invoke(verb, input, idempotency_key).await?;
        let events = out
            .events
            .into_iter()
            .map(|d| self.instance.envelope(actor, d))
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
