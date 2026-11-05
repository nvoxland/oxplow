//! The scopes' **operations** (`.context/commands.md` "Scopes"): the
//! native behavior behind the commands extensions declare. A scope
//! (`work_items.write`) is in the domain's catalog; each of its
//! operations (`transition`) is registered here with what only Rust can
//! supply — the input schema of the type its handler reads, the handler,
//! whether it returns an inverse, a per-input confirmation, a check
//! before the transaction.
//!
//! A manifest's command names one (`scope: work_items.write`, `op:
//! transition`) and says the rest — summary, confirmation, ui —
//! so oxplow's own commands are declared like any extension's
//! (`extensions/oxplow-foundation`), and any extension may declare one
//! over the same operation.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_domain::{Atomicity, CommandError, CommandSpec, Confirm};
use serde_json::Value;

use super::{Command, ConfirmFor, Handler, Precheck};

/// One operation of a scope.
pub struct Op {
    /// Its scope: `work_items.write`.
    pub scope: String,
    /// Its name within the scope: `transition`.
    pub name: String,
    /// The input its handler reads.
    pub input_schema: Value,
    /// The result it returns, when it declares one (`Op::returns`).
    pub output_schema: Option<Value>,
    /// It returns the call that undoes it.
    pub undoable: bool,
    pub handler: Handler,
    confirm_for: Option<Arc<ConfirmFor>>,
    precheck: Option<Arc<Precheck>>,
    /// Input fields its runs' records leave out (`CommandSpec::unrecorded`).
    unrecorded: Vec<String>,
    /// The weakest confirmation a command over it may declare
    /// (`Op::confirm_at_least`): a declaration asks this much or more.
    confirm_at_least: Confirm,
}

impl Op {
    pub fn new(
        scope: &str,
        name: &str,
        input_schema: Value,
        undoable: bool,
        handler: Handler,
    ) -> Self {
        Self {
            scope: scope.into(),
            name: name.into(),
            input_schema,
            output_schema: None,
            undoable,
            handler,
            confirm_for: None,
            precheck: None,
            unrecorded: Vec::new(),
            confirm_at_least: Confirm::Never,
        }
    }

    /// It returns a `T`: its commands declare `T`'s schema as their output,
    /// so an MCP client gets the result as structured content.
    pub fn returns<T: schemars::JsonSchema>(mut self) -> Self {
        self.output_schema = Some(super::util::schema::<T>());
        self
    }

    /// The weakest confirmation a command over it may declare: one that
    /// asks less is refused.
    pub fn confirm_at_least(mut self, floor: Confirm) -> Self {
        self.confirm_at_least = floor;
        self
    }

    /// `scope/op`.
    pub fn id(&self) -> String {
        format!("{}/{}", self.scope, self.name)
    }

    /// `spec` asks at least as much as this operation's floor, and isn't
    /// a job's: a durable job's model calls only its extension's own
    /// handlers.
    fn within_floor(&self, spec: &CommandSpec) -> Result<(), CommandError> {
        if spec.job {
            return Err(CommandError::Invalid {
                field: Some("/job".into()),
                message: format!(
                    "`{}` is over `{}`: a job calls only its extension's own handlers (`entry:`)",
                    spec.id,
                    self.id()
                ),
            });
        }
        if spec.confirm < self.confirm_at_least {
            return Err(CommandError::Invalid {
                field: Some("/confirm".into()),
                message: format!(
                    "`{}` is confirmed `{}` at least; `{}` declares `{}`",
                    self.id(),
                    confirm_name(self.confirm_at_least),
                    spec.id,
                    confirm_name(spec.confirm)
                ),
            });
        }
        Ok(())
    }

    /// It decides per input whether a person confirms (`oxplow.config.set`
    /// on a human-only key), over what the declaration says.
    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Self {
        self.confirm_for = Some(f);
        self
    }

    /// Its check before the transaction (`super::Precheck`).
    /// Its runs' records leave out `fields` of the input (a file's
    /// content), keeping their size.
    pub fn with_unrecorded(mut self, fields: &[&str]) -> Self {
        self.unrecorded = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    pub fn with_precheck(mut self, f: Arc<Precheck>) -> Self {
        self.precheck = Some(f);
        self
    }

    /// The command `spec` declares, backed by this operation: its input
    /// schema (with the declaration's example), undo and atomicity are the
    /// operation's, its access the
    /// scope's, and the scope is among its needs. Its
    /// `confirm` stays within the operation's floor, and it isn't a job's.
    pub fn command(&self, mut spec: CommandSpec) -> Result<Command, CommandError> {
        let access = oxplow_domain::scope::scope(&self.scope)
            .ok_or_else(|| CommandError::Failed {
                message: format!("no scope `{}`", self.scope),
            })?
            .access;
        self.within_floor(&spec)?;
        // The declaration's example input (`example:`) is kept over the
        // operation's schema.
        let example = spec.input_schema.get("examples").cloned();
        spec.input_schema = self.input_schema.clone();
        if let (Some(example), Some(schema)) = (example, spec.input_schema.as_object_mut()) {
            schema.insert("examples".into(), example);
        }
        spec.output_schema = self.output_schema.clone();
        spec.undoable = self.undoable;
        spec.atomicity = match self.handler {
            Handler::Tx(_) => Atomicity::Tx,
            Handler::External(_) => Atomicity::External,
            Handler::Compose(_) => Atomicity::Dispatch,
        };
        spec.access = access;
        spec.unrecorded = self.unrecorded.clone();
        spec.op = Some(oxplow_domain::OpRef {
            scope: self.scope.clone(),
            op: self.name.clone(),
        });
        if !spec.needs.contains(&self.scope) {
            spec.needs.insert(0, self.scope.clone());
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

/// A confirmation as a manifest spells it.
fn confirm_name(confirm: Confirm) -> &'static str {
    match confirm {
        Confirm::Never => "never",
        Confirm::Always => "always",
        Confirm::Destructive => "destructive",
    }
}

/// The registered operations, by scope and name.
#[derive(Default)]
pub struct Ops(BTreeMap<(String, String), Arc<Op>>);

impl Ops {
    pub fn add(&mut self, op: Op) -> Result<(), CommandError> {
        if oxplow_domain::scope::scope(&op.scope).is_none() {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("`{}` isn't a scope", op.scope),
            });
        }
        let key = (op.scope.clone(), op.name.clone());
        if self.0.contains_key(&key) {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("`{}` already has the op `{}`", key.0, key.1),
            });
        }
        self.0.insert(key, Arc::new(op));
        Ok(())
    }

    pub fn get(&self, scope: &str, op: &str) -> Option<Arc<Op>> {
        self.0.get(&(scope.to_string(), op.to_string())).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{Access, Lifecycle};
    use serde_json::json;

    fn op() -> Op {
        Op::new(
            "lsp.install",
            "install_server",
            json!({ "type": "object" }),
            false,
            Handler::External(Arc::new(|_, _| {
                Box::pin(async { Ok(super::super::HandlerOutput::default()) })
            })),
        )
    }

    fn spec(job: bool, confirm: Confirm) -> CommandSpec {
        CommandSpec {
            output_schema: None,
            id: "acme.lsp.install".into(),
            summary: "Install.".into(),
            input_schema: json!({}),
            job,
            confirm,
            confirm_says: None,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::External,
            access: Access::Write,
            needs: vec![],
            ui: None,
            op: None,
            unrecorded: vec![],
        }
    }

    /// An operation's floor bounds every command declared over it, from
    /// any extension: a declaration asks as much or more, never less, and
    /// a job never calls one (a job calls its extension's own handlers).
    #[test]
    fn a_declaration_asks_at_least_its_ops_floor_and_isnt_a_jobs() {
        let floored = op().confirm_at_least(Confirm::Always);
        floored.command(spec(false, Confirm::Always)).unwrap();
        floored.command(spec(false, Confirm::Destructive)).unwrap();
        let err = floored
            .command(spec(false, Confirm::Never))
            .map(|_| ())
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/confirm"
                && message.contains("is confirmed `always` at least")
                && message.contains("declares `never`")),
            "{err:?}"
        );
        let err = op()
            .command(spec(true, Confirm::Never))
            .map(|_| ())
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/job"),
            "{err:?}"
        );
    }
}
