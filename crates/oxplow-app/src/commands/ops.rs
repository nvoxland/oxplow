//! The scopes' **operations** (`.context/commands.md` "Scopes"): the
//! native behavior behind the commands extensions declare. A scope
//! (`work_items.write`) is in the domain's catalog; each of its
//! operations (`transition`) is registered here with what only Rust can
//! supply — the input schema of the type its handler reads, the handler,
//! whether it returns an inverse, a per-input confirmation, a check
//! before the transaction.
//!
//! A manifest's command names one (`scope: work_items.write`, `op:
//! transition`) and says the rest — summary, invokers, confirmation, ui —
//! so oxplow's own commands are declared like any extension's
//! (`extensions/oxplow-foundation`), and any extension may declare one
//! over the same operation.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_domain::{Atomicity, CommandError, CommandSpec, Confirm, Invokers};
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
    /// It returns the call that undoes it.
    pub undoable: bool,
    pub handler: Handler,
    confirm_for: Option<Arc<ConfirmFor>>,
    precheck: Option<Arc<Precheck>>,
    /// Input fields its runs' records leave out (`CommandSpec::unrecorded`).
    unrecorded: Vec<String>,
    /// Who may run a command over it, at most — its **floor**: a
    /// declaration admits these or fewer (`Op::open_to`). Everyone until
    /// the operation says otherwise; `Ops::fully_open` lists those, pinned
    /// by a test, so leaving one open is a reviewed choice.
    open_to: Invokers,
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
            undoable,
            handler,
            confirm_for: None,
            precheck: None,
            unrecorded: Vec::new(),
            open_to: Invokers::ALL,
            confirm_at_least: Confirm::Never,
        }
    }

    /// Who may run a command over it, at most (`Invokers::HUMAN_ONLY`: a
    /// person's; `Invokers::NO_AGENT`: a person's, directly or through a
    /// lens). A declaration that admits anyone else is refused.
    pub fn open_to(mut self, floor: Invokers) -> Self {
        self.open_to = floor;
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

    /// Whether anyone may run it with no confirmation — no floor at all.
    pub fn is_fully_open(&self) -> bool {
        self.open_to == Invokers::ALL && self.confirm_at_least == Confirm::Never
    }

    /// `spec` admits no more than this operation's floor, and asks at
    /// least as much.
    fn within_floor(&self, spec: &CommandSpec) -> Result<(), CommandError> {
        if !spec.invokers.within(&self.open_to) {
            return Err(CommandError::Invalid {
                field: Some("/invokers".into()),
                message: format!(
                    "`{}` is open to {} at most; `{}` admits {}",
                    self.id(),
                    self.open_to.names(),
                    spec.id,
                    spec.invokers.names()
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
    /// schema, undo and atomicity are the operation's, its access the
    /// scope's, and the scope is among its needs. Its
    /// `invokers` and `confirm` stay within the operation's floor.
    pub fn command(&self, mut spec: CommandSpec) -> Result<Command, CommandError> {
        let access = oxplow_domain::scope::scope(&self.scope)
            .ok_or_else(|| CommandError::Failed {
                message: format!("no scope `{}`", self.scope),
            })?
            .access;
        self.within_floor(&spec)?;
        spec.input_schema = self.input_schema.clone();
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

    /// The operations with no floor, as `scope/op`, sorted.
    pub fn fully_open(&self) -> Vec<String> {
        self.0
            .values()
            .filter(|op| op.is_fully_open())
            .map(|op| op.id())
            .collect()
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

    fn spec(invokers: Invokers, confirm: Confirm) -> CommandSpec {
        CommandSpec {
            id: "acme.lsp.install".into(),
            summary: "Install.".into(),
            input_schema: json!({}),
            invokers,
            confirm,
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
    /// any extension: a declaration narrows who may run it and asks more,
    /// never the reverse.
    #[test]
    fn a_declaration_narrows_an_ops_floor_and_never_widens_it() {
        let floored = op()
            .open_to(Invokers::HUMAN_ONLY)
            .confirm_at_least(Confirm::Always);
        floored
            .command(spec(Invokers::HUMAN_ONLY, Confirm::Always))
            .unwrap();
        floored
            .command(spec(Invokers::HUMAN_ONLY, Confirm::Destructive))
            .unwrap();
        let err = floored
            .command(spec(Invokers::ALL, Confirm::Always))
            .map(|_| ())
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/invokers"
                && message.contains("`lsp.install/install_server` is open to human at most")
                && message.contains("`acme.lsp.install` admits human, agent, lens")),
            "{err:?}"
        );
        let err = floored
            .command(spec(Invokers::HUMAN_ONLY, Confirm::Never))
            .map(|_| ())
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/confirm"
                && message.contains("is confirmed `always` at least")
                && message.contains("declares `never`")),
            "{err:?}"
        );
    }

    /// Until an operation says otherwise it has no floor, and the
    /// registry can name those.
    #[test]
    fn an_op_without_a_floor_is_fully_open_and_listed() {
        assert!(op().is_fully_open());
        assert!(!op().open_to(Invokers::NO_AGENT).is_fully_open());
        assert!(!op().confirm_at_least(Confirm::Destructive).is_fully_open());
        let mut ops = Ops::default();
        ops.add(op()).unwrap();
        ops.add(
            Op::new(
                "bookmarks.write",
                "set",
                json!({}),
                true,
                Handler::Tx(Arc::new(|_, _| Ok(super::super::HandlerOutput::default()))),
            )
            .open_to(Invokers::NO_AGENT),
        )
        .unwrap();
        assert_eq!(ops.fully_open(), vec!["lsp.install/install_server"]);
    }
}
