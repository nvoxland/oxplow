//! The host capabilities' **operations** (`.context/commands.md` "Host
//! capabilities"): the native behavior behind the commands extensions
//! declare. A capability is a scope (`work_items.write`) in the domain's
//! catalog; each of its operations (`transition`) is registered here with
//! what only Rust can supply — the input schema of the type its handler
//! reads, the handler, whether it returns an inverse, a per-input
//! confirmation, a check before the transaction.
//!
//! A manifest's command names one (`capability: work_items.write`, `op:
//! transition`) and says the rest — summary, invokers, confirmation, ui —
//! so oxplow's own commands are declared like any extension's
//! (`extensions/oxplow-foundation`), and any extension may declare one
//! over the same operation.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_domain::{Atomicity, CommandEffect, CommandError, CommandSpec};
use serde_json::Value;

use super::{Command, ConfirmFor, Handler, Precheck};

/// One operation of a host capability.
pub struct Op {
    /// Its scope: `work_items.write`.
    pub capability: String,
    /// Its name within the scope: `transition`.
    pub name: String,
    /// The input its handler reads.
    pub input_schema: Value,
    /// It returns the call that undoes it.
    pub undoable: bool,
    pub handler: Handler,
    confirm_for: Option<Arc<ConfirmFor>>,
    precheck: Option<Arc<Precheck>>,
}

impl Op {
    pub fn new(
        capability: &str,
        name: &str,
        input_schema: Value,
        undoable: bool,
        handler: Handler,
    ) -> Self {
        Self {
            capability: capability.into(),
            name: name.into(),
            input_schema,
            undoable,
            handler,
            confirm_for: None,
            precheck: None,
        }
    }

    /// It decides per input whether a person confirms (`oxplow.config.set`
    /// on a human-only key), over what the declaration says.
    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Self {
        self.confirm_for = Some(f);
        self
    }

    /// Its check before the transaction (`super::Precheck`).
    pub fn with_precheck(mut self, f: Arc<Precheck>) -> Self {
        self.precheck = Some(f);
        self
    }

    /// The command `spec` declares, backed by this operation: its input
    /// schema, undo and atomicity are the operation's, its effect the
    /// capability's class, and the capability is among its needs.
    pub fn command(&self, mut spec: CommandSpec) -> Result<Command, CommandError> {
        let class = oxplow_domain::host_capability::host_capability(&self.capability)
            .ok_or_else(|| CommandError::Failed {
                message: format!("no host capability `{}`", self.capability),
            })?
            .class;
        spec.input_schema = self.input_schema.clone();
        spec.undoable = self.undoable;
        spec.atomicity = match self.handler {
            Handler::Tx(_) => Atomicity::Tx,
            Handler::External(_) => Atomicity::External,
            Handler::Compose(_) => Atomicity::Dispatch,
        };
        spec.effect = effect_of(class);
        if !spec.needs.contains(&self.capability) {
            spec.needs.insert(0, self.capability.clone());
        }
        let mut command = Command::new(spec, self.handler.clone())?;
        if let Some(f) = &self.confirm_for {
            command = command.with_confirm_for(f.clone())?;
        }
        if let Some(f) = &self.precheck {
            command = command.with_precheck(f.clone());
        }
        Ok(command)
    }
}

/// A command's effect, from the class of the capability behind it: only
/// a record or a write is audited.
pub fn effect_of(class: oxplow_domain::host_capability::EffectClass) -> CommandEffect {
    use oxplow_domain::host_capability::EffectClass;
    match class {
        EffectClass::View | EffectClass::Read => CommandEffect::Read,
        EffectClass::Record => CommandEffect::Record,
        EffectClass::Write => CommandEffect::Write,
    }
}

/// The registered operations, by capability and name.
#[derive(Default)]
pub struct Ops(BTreeMap<(String, String), Arc<Op>>);

impl Ops {
    pub fn add(&mut self, op: Op) -> Result<(), CommandError> {
        if oxplow_domain::host_capability::host_capability(&op.capability).is_none() {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("`{}` isn't a host capability in the catalog", op.capability),
            });
        }
        let key = (op.capability.clone(), op.name.clone());
        if self.0.contains_key(&key) {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("`{}` already has the op `{}`", key.0, key.1),
            });
        }
        self.0.insert(key, Arc::new(op));
        Ok(())
    }

    pub fn get(&self, capability: &str, op: &str) -> Option<Arc<Op>> {
        self.0
            .get(&(capability.to_string(), op.to_string()))
            .cloned()
    }
}
