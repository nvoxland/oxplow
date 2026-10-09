//! An external provider's effort policy: [`ExternalEffortPolicy`] is the
//! [`EffortPolicy`] the effort-policy dispatcher calls while the instance
//! is the project's choice. Each event it's offered is one `react`
//! invoke, `{ event }` (a [`PolicyEvent`], anchors included); it answers as
//! an effect's script does — `{ commands: [{ name, input }] }` or
//! `{ skip: "why" }` — and core runs the commands as the policy's own
//! effect actor, through every gate an agent meets. It emits nothing of
//! its own. What it reads of oxplow it reads through `host/call`
//! (`needs: [sql.read]`).

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::effort_policy::{EffortPolicy, EffortPolicyRegistry, PolicyEvent};
use oxplow_domain::{CommandCall, DomainError, InputValidator};
use serde_json::{json, Value};

use super::registry::{CapabilityHost, Instance};

/// The verb the dispatcher calls.
const REACT: &str = "react";

/// The effort policy's side of the host: a started instance is a policy
/// in `Services.effort_policies`, under its id.
pub struct EffortPolicyHost(pub Arc<EffortPolicyRegistry>);

impl EffortPolicyHost {
    pub const CAPABILITY: &'static str = "effort_policy";
}

impl CapabilityHost for EffortPolicyHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.0.has(id)
    }

    fn admit(&self, instance: &Arc<Instance>) -> Result<Value, String> {
        let declared = &instance.declared;
        let react = declared
            .commands
            .iter()
            .find(|c| c.name == REACT)
            .ok_or("it declares no `react` verb")?;
        let input = InputValidator::compile(&react.input_schema)
            .map_err(|e| format!("verb `react`: {e}"))?;
        let features = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .map(|c| c.features.clone())
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        self.0.register(Arc::new(ExternalEffortPolicy {
            instance: instance.clone(),
            input,
        }));
        Ok(features)
    }

    fn retire(&self, id: &str) {
        self.0.unregister(id);
    }
}

pub struct ExternalEffortPolicy {
    instance: Arc<Instance>,
    /// `react`'s declared input schema, compiled.
    input: InputValidator,
}

#[async_trait]
impl EffortPolicy for ExternalEffortPolicy {
    fn id(&self) -> &str {
        &self.instance.id
    }

    async fn react(&self, event: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError> {
        let failed = |message: String| {
            DomainError::Invariant(format!("effort policy `{}`: {message}", self.instance.name))
        };
        let input = json!({ "event": event });
        self.input
            .check(&input)
            .map_err(|e| failed(format!("its `react` schema refuses the event: {e}")))?;
        // One key per event: a redelivered event is the same call, which a
        // provider keeping `idempotent_writes` answers once.
        let out = self
            .instance
            .invoke(REACT, input, Some(format!("react:{}", event.id)))
            .await
            .map_err(|e| failed(e.to_string()))?;
        if !out.events.is_empty() {
            return Err(failed(
                "it returned events, and an effort policy emits none".into(),
            ));
        }
        calls_of(out.result).map_err(failed)
    }
}

/// The calls a `react` answer composes; none for a skip.
fn calls_of(result: Value) -> Result<Vec<CommandCall>, String> {
    match crate::effects::reaction(result)? {
        crate::effects::Reaction::Skip(_) => Ok(Vec::new()),
        crate::effects::Reaction::Run { events, .. } if !events.is_empty() => {
            Err("it composed events, and an effort policy emits none".into())
        }
        crate::effects::Reaction::Run { calls, .. } => Ok(calls),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A policy answers as an effect's script does: commands to run, or a
    /// skip; events are refused, and so is anything else.
    #[test]
    fn a_react_answer_is_commands_or_a_skip() {
        let calls = calls_of(json!({
            "commands": [{ "name": "oxplow.effort.open", "input": { "thread": "thread:thr1" } }]
        }))
        .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "oxplow.effort.open");
        assert_eq!(calls[0].input["thread"], "thread:thr1");
        assert!(calls_of(json!({ "skip": "not mine" })).unwrap().is_empty());
        let err = calls_of(json!({
            "commands": [],
            "events": [{ "type": "x.y", "payload": {} }]
        }))
        .unwrap_err();
        assert!(err.contains("emits none"), "{err}");
        assert!(calls_of(json!({ "nope": 1 })).is_err());
    }
}
