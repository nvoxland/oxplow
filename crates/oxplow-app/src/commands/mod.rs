//! The command bus: the one write path (`.context/commands.md`;
//! `.context/target-architecture.md` §7).
//!
//! Every command runs the same pipeline — validate the input against the
//! spec's schema, check the invoker, apply the agent policy, require a
//! person's confirmation when the spec says so, run the handler, and
//! record `command.executed` in the event log plus a `command_audit` row
//! **in one transaction**. `undo(audit_id)` runs the recorded inverse
//! through the same pipeline.
//!
//! A handler is either `Tx` (runs inside the bus's transaction, so its
//! writes, the event and the audit commit together) or `External` (runs
//! outside it, against a system the bus doesn't own — a VCS, a provider
//! process, a gauge script — and is audited after it returns).
//! [`CommandBus::external_commands`] is pinned by a test, so every
//! `External` command is a reviewed choice, not a shortcut.

pub mod comment;
pub mod compose;
pub mod config_commands;
pub mod dashboard;
pub mod effort;
pub mod effort_report;
pub mod lens;
pub mod metric;
pub mod note;
pub mod reasoning;
pub mod review;
mod steps;
pub mod stream;
pub mod test_runs;
pub mod thread;
pub mod vcs;
pub mod work_item;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use oxplow_db::command_audit_store::{insert_tx, mark_undone_tx, set_event_id_tx, NewCommandAudit};
use oxplow_db::event_log_store::append_tx;
use oxplow_db::proposal_store::{self, NewProposal};
use oxplow_db::{
    CommandAudit, Database, ProposalDecision, SqliteCommandAuditStore, SqliteEventLogStore,
    SqliteProposalStore,
};
use oxplow_domain::events::schema::{
    CommandApproved, CommandApprovedV1, CommandDeclined, CommandDeclinedV1, CommandExecuted,
    CommandExecutedV1, CommandOutcome as Outcome, CommandProposed, CommandProposedV1,
};
use oxplow_domain::refs::build::{command_ref, proposal_ref};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec, Envelope,
    InputValidator, Preview,
};
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value;

use crate::agent_policy::AgentPolicy;
use crate::event_pump::EventPump;

/// What a handler produced.
#[derive(Default)]
pub struct HandlerOutput {
    /// Returned to the caller (`run_command`'s result).
    pub result: Value,
    /// The call that undoes this run; `None` for a command that declares
    /// `undoable: false`, or when there is nothing to undo.
    pub inverse: Option<CommandCall>,
    /// Domain events the run produced, appended after `command.executed`
    /// in the same transaction. An `External` handler's own transactions
    /// log their events themselves; this is for `Tx` handlers.
    pub events: Vec<Envelope>,
    /// Runs once the run has committed: the in-memory broadcast that wakes
    /// the UI (`OxplowEvent`), and side effects outside the database that
    /// mirror the committed change (`config.set` writes project.yaml here).
    /// Never a database write, and it must tolerate failure — the run is
    /// already recorded. The handler itself must stay pure: it can run
    /// more than once (`Database::transaction` retries on SQLITE_BUSY).
    pub after_commit: Option<Box<dyn FnOnce() + Send + Sync>>,
}

/// One child of a composite run (`CommandBus::run_nested`): what ran,
/// with what, what it returned and how it undoes.
#[derive(Debug, Clone, Serialize)]
pub struct NestedChild {
    pub name: String,
    pub input: Value,
    pub result: Value,
    pub inverse: Option<CommandCall>,
}

/// What a composite's children produced, for the parent's `HandlerOutput`.
pub struct NestedOutcome {
    pub children: Vec<NestedChild>,
    /// The children's inverses reversed, as a `command.sequence`; `None`
    /// when a child has none.
    pub inverse: Option<CommandCall>,
    pub events: Vec<Envelope>,
    pub after_commit: Option<Box<dyn FnOnce() + Send + Sync>>,
}

/// What a `Tx` handler runs with: the run's transaction, the actor, and
/// the context its store cores log events through — the actor's `source`,
/// caused by this run's `command.executed` (whose id is fixed before the
/// handler runs, so the cause can be named; `command.executed` itself is
/// appended after the handler, so a core's events precede it in `seq`).
pub struct TxCtx<'a> {
    pub conn: &'a rusqlite::Connection,
    pub actor: &'a Actor,
    pub events: oxplow_db::EventCtx<'a>,
    /// May this run open an effort (claim the worktree)? False only for
    /// an agent thread that isn't its stream's writer; a `Record`
    /// handler checks it with [`TxCtx::claim`].
    pub may_claim: bool,
    /// Step 4's answer: a person confirmed this call. A handler that
    /// learns only while running that a confirmation is needed — a
    /// composite whose child asks (`CommandBus::run_nested`) — checks it
    /// and raises `NeedsConfirmation`, which the bus treats as step 4
    /// would: rolled back, nothing audited.
    pub confirmed: bool,
    /// Step 3's gate: whether the agent's thread may write; `None` for a
    /// person, the system, or a command the gate doesn't apply to. A
    /// composite applies it to each child it runs.
    pub may_write: Option<bool>,
    /// How many composites this run is inside: 0 for the bus's own call,
    /// one more for each `run_nested` level. Past [`MAX_NESTING`] a
    /// composition is refused rather than recursing until the stack
    /// overflows (a command that composes itself).
    pub depth: usize,
}

/// The deepest a composite may nest (`command.sequence` and extension
/// commands composing one another).
pub const MAX_NESTING: usize = 8;

impl TxCtx<'_> {
    /// Refuse the run when it opened an effort (`opened`) and the actor
    /// may not claim. The bus rolls the transaction back and audits the
    /// refusal as denied.
    pub fn claim(&self, opened: bool, what: &str) -> Result<(), CommandError> {
        if opened && !self.may_claim {
            return Err(CommandError::Denied {
                reason: format!(
                    "{what} opens an effort — a claim on the worktree — and only the \
                     stream's writer thread may claim; file or edit it without \
                     `in_progress`, or run this from the writer thread"
                ),
            });
        }
        Ok(())
    }
}

pub type TxHandler = dyn Fn(&TxCtx<'_>, Value) -> Result<HandlerOutput, CommandError> + Send + Sync;
pub type ExternalFuture = Pin<Box<dyn Future<Output = Result<HandlerOutput, CommandError>> + Send>>;
pub type ExternalHandler = dyn Fn(Actor, Value) -> ExternalFuture + Send + Sync;

pub enum Handler {
    /// Runs inside the bus's transaction.
    Tx(Arc<TxHandler>),
    /// Runs against a system the bus doesn't own; the bus audits it after
    /// it returns.
    External(Arc<ExternalHandler>),
    /// Decides per input which of the two it is (P7.A1): the `work_item.*`
    /// verbs route by the ref's provider — oxplow's items in the
    /// transaction, another provider's through its process.
    Dispatch(Dispatch),
    /// A composite (`compose.rs`): says which calls an input runs; the bus
    /// runs them in its transaction when every one can, else as steps.
    Compose(Arc<compose::Compose>),
}

/// Where a `Dispatch` command's run goes for one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The `Tx` handler, in the bus's transaction.
    Tx,
    /// The `External` handler, against the named system (`provider
    /// \`linear\``) — what a refusal to compose it names.
    External(String),
}

type Router = dyn Fn(&Value) -> Result<Route, CommandError> + Send + Sync;

/// A `Dispatch` handler: the route decision and the two handlers it
/// picks between. The route runs after the input has passed the schema,
/// so it sees a well-formed input; its error is the caller's (`Invalid`).
pub struct Dispatch {
    pub route: Arc<Router>,
    pub tx: Arc<TxHandler>,
    pub external: Arc<ExternalHandler>,
}

/// A handler with its route decided: what step 5 runs.
#[derive(Clone)]
enum Resolved {
    Tx(Arc<TxHandler>),
    External(Arc<ExternalHandler>),
    /// A composite with a call outside the transaction: its checked steps.
    Steps(Arc<steps::Plan>),
}

/// Step 1's routing: decided from the input alone, or a composite, routed
/// once the actor has been admitted (composing reads the database and may
/// run an extension's script).
enum Routing {
    Ready(Resolved),
    Composite(Arc<compose::Compose>),
}

/// A command with its handler resolved for the input being run.
#[derive(Clone, Copy)]
struct Prepared<'a> {
    command: &'a Arc<Command>,
    resolved: &'a Resolved,
}

type ConfirmFor = dyn Fn(&Value) -> oxplow_domain::Confirm + Send + Sync;

/// A registered command: its spec, compiled input schema and handler.
pub struct Command {
    pub spec: CommandSpec,
    pub handler: Handler,
    validator: InputValidator,
    /// Per-input confirmation, when the spec's `confirm` depends on the
    /// input (`config.set` on a human-only key). Overrides `spec.confirm`.
    confirm_for: Option<Arc<ConfirmFor>>,
}

impl Command {
    pub fn new(spec: CommandSpec, handler: Handler) -> Result<Self, CommandError> {
        CommandSpec::validate_name(&spec.name)?;
        let declared = match handler {
            Handler::Tx(_) => Atomicity::Tx,
            Handler::External(_) => Atomicity::External,
            Handler::Dispatch(_) | Handler::Compose(_) => Atomicity::Dispatch,
        };
        if declared != spec.atomicity {
            return Err(CommandError::Invalid {
                field: Some("/atomicity".into()),
                message: format!(
                    "`{}` declares {:?} but its handler is {:?}",
                    spec.name, spec.atomicity, declared
                ),
            });
        }
        if spec.confirm.required() {
            Self::may_ask(&spec)?;
        }
        let validator = InputValidator::compile(&spec.input_schema)?;
        Ok(Self {
            spec,
            handler,
            validator,
            confirm_for: None,
        })
    }

    /// Its confirmation decided per input. A `Read` command may not have
    /// one (see [`Self::may_ask`]).
    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Result<Self, CommandError> {
        Self::may_ask(&self.spec)?;
        self.confirm_for = Some(f);
        Ok(self)
    }

    /// A command that only reads is never asked about: it runs without a
    /// record, so nothing would resolve its proposal or carry its
    /// confirmation.
    fn may_ask(spec: &CommandSpec) -> Result<(), CommandError> {
        if spec.effect == oxplow_domain::CommandEffect::Read {
            return Err(CommandError::Invalid {
                field: Some("/confirm".into()),
                message: format!(
                    "`{}` only reads, so it can't need a confirmation",
                    spec.name
                ),
            });
        }
        Ok(())
    }

    fn confirm(&self, input: &Value) -> oxplow_domain::Confirm {
        match &self.confirm_for {
            Some(f) => f(input),
            None => self.spec.confirm,
        }
    }

    /// The handler for `input`: a `Dispatch` command's route decided (its
    /// error is the caller's), the others as they are.
    fn resolve(&self, input: &Value) -> Result<Routing, CommandError> {
        Ok(Routing::Ready(match &self.handler {
            Handler::Tx(h) => Resolved::Tx(h.clone()),
            Handler::External(h) => Resolved::External(h.clone()),
            Handler::Dispatch(d) => match (d.route)(input)? {
                Route::Tx => Resolved::Tx(d.tx.clone()),
                Route::External(_) => Resolved::External(d.external.clone()),
            },
            Handler::Compose(c) => return Ok(Routing::Composite(c.clone())),
        }))
    }
}

/// May this agent thread write? (`Thread::status.is_writer()` in
/// production.) The bus asks before an agent runs a `Write` command.
pub type WriteGate =
    Arc<dyn Fn(oxplow_domain::ThreadId) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

/// What a run is, beyond the call itself: the undo of an audited run, or
/// the approval of a proposal. Either is marked in the run's own
/// transaction (a `Tx` handler) or claimed before it (an `External` one).
impl crate::extensions::RunningCommands for CommandBus {
    fn input_schema(&self, name: &str) -> Option<Value> {
        CommandBus::input_schema(self, name)
    }

    fn namespace_owner(&self, namespace: &str) -> Option<String> {
        CommandBus::namespace_owner(self, namespace)
    }
}

/// The registered commands by name, and who holds each namespace
/// registered whole (`register_namespace`); a namespace with commands but
/// no owner is oxplow's.
#[derive(Default)]
struct Registry {
    commands: BTreeMap<String, Arc<Command>>,
    owners: BTreeMap<String, String>,
}

impl Registry {
    fn owner(&self, namespace: &str) -> Option<String> {
        if let Some(owner) = self.owners.get(namespace) {
            return Some(owner.clone());
        }
        let prefix = format!("{namespace}.");
        self.commands
            .keys()
            .any(|n| n.starts_with(&prefix))
            .then(|| "oxplow".to_string())
    }

    fn get(&self, name: &str) -> Option<&Arc<Command>> {
        self.commands.get(name)
    }
}

/// Step 3's answers for a run, as its `TxCtx` carries them: may it claim
/// the worktree, and may the agent's thread write (`None` when the gate
/// doesn't apply).
#[derive(Debug, Clone, Copy)]
struct Gates {
    may_claim: bool,
    may_write: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
enum RunOrigin {
    Call,
    Undo(i64),
    Approval(i64),
}

pub struct CommandBus {
    db: Database,
    log: SqliteEventLogStore,
    audit: SqliteCommandAuditStore,
    proposals: SqliteProposalStore,
    policy: Arc<AgentPolicy>,
    pump: Arc<EventPump>,
    commands: RwLock<Registry>,
    write_gate: Option<WriteGate>,
}

impl CommandBus {
    pub fn new(
        db: Database,
        log: SqliteEventLogStore,
        policy: Arc<AgentPolicy>,
        pump: Arc<EventPump>,
    ) -> Self {
        Self {
            audit: SqliteCommandAuditStore::new(db.clone()),
            proposals: SqliteProposalStore::new(db.clone()),
            db,
            log,
            policy,
            pump,
            commands: RwLock::new(Registry::default()),
            write_gate: None,
        }
    }

    /// Consult `gate` before an agent thread runs a `Write` command: a
    /// queued or closed thread can read but not change anything.
    pub fn with_write_gate(mut self, gate: WriteGate) -> Self {
        self.write_gate = Some(gate);
        self
    }

    /// Register a command. A second command of the same name is refused:
    /// two handlers for one name is a bug, not an override.
    pub fn register(&self, command: Command) -> Result<(), CommandError> {
        let mut registry = self.commands.write();
        let namespace = command.spec.name.split('.').next().unwrap_or_default();
        if let Some(owner) = registry.owners.get(namespace) {
            return Err(CommandError::Invalid {
                field: Some("/name".into()),
                message: format!("the command namespace `{namespace}` is {owner}'s"),
            });
        }
        if registry.commands.contains_key(&command.spec.name) {
            return Err(CommandError::Invalid {
                field: Some("/name".into()),
                message: format!("command `{}` is already registered", command.spec.name),
            });
        }
        registry
            .commands
            .insert(command.spec.name.clone(), Arc::new(command));
        Ok(())
    }

    /// Register `commands` as the whole of `namespace`, held by `owner`
    /// (`extension:<name>`, `provider:<instance>`) — all of them or none,
    /// under one lock: refused when the namespace is taken (by another
    /// owner, or by oxplow's own commands) or a command isn't under it.
    pub fn register_namespace(
        &self,
        namespace: &str,
        owner: &str,
        commands: Vec<Command>,
    ) -> Result<(), CommandError> {
        let mut registry = self.commands.write();
        if let Some(held) = registry.owner(namespace) {
            return Err(CommandError::Invalid {
                field: None,
                message: format!("the command namespace `{namespace}` is already {held}'s"),
            });
        }
        let prefix = format!("{namespace}.");
        if let Some(stray) = commands.iter().find(|c| !c.spec.name.starts_with(&prefix)) {
            return Err(CommandError::Invalid {
                field: Some("/name".into()),
                message: format!("`{}` isn't under `{namespace}`", stray.spec.name),
            });
        }
        registry
            .owners
            .insert(namespace.to_string(), owner.to_string());
        for command in commands {
            registry
                .commands
                .insert(command.spec.name.clone(), Arc::new(command));
        }
        Ok(())
    }

    /// Remove `namespace` and every command under it (an extension
    /// disabled, a provider instance stopped); returns their names.
    pub fn unregister_namespace(&self, namespace: &str) -> Vec<String> {
        let prefix = format!("{namespace}.");
        let mut registry = self.commands.write();
        registry.owners.remove(namespace);
        let names: Vec<String> = registry
            .commands
            .keys()
            .filter(|n| n.starts_with(&prefix))
            .cloned()
            .collect();
        for n in &names {
            registry.commands.remove(n);
        }
        names
    }

    /// Who holds `namespace`: its registered owner, `oxplow` for core
    /// commands, `None` when nothing is under it.
    pub fn namespace_owner(&self, namespace: &str) -> Option<String> {
        self.commands.read().owner(namespace)
    }

    /// The event types the log accepts.
    pub fn event_schemas(&self) -> &Arc<oxplow_domain::EventSchemaRegistry> {
        self.log.schemas()
    }

    pub fn audit_store(&self) -> &SqliteCommandAuditStore {
        &self.audit
    }

    pub fn proposal_store(&self) -> &SqliteProposalStore {
        &self.proposals
    }

    /// The specs `actor` may invoke, by name.
    pub fn list(&self, actor: &Actor) -> Vec<CommandSpec> {
        self.commands
            .read()
            .commands
            .values()
            .filter(|c| c.spec.invokers.allows(actor.invoker()))
            .map(|c| c.spec.clone())
            .collect()
    }

    pub fn spec(&self, name: &str) -> Option<CommandSpec> {
        self.commands.read().get(name).map(|c| c.spec.clone())
    }

    /// A registered command's input schema: what an extension's launcher
    /// command entry is checked against (`extensions::RunningCommands`).
    pub fn input_schema(&self, name: &str) -> Option<Value> {
        self.commands
            .read()
            .get(name)
            .map(|c| c.spec.input_schema.clone())
    }

    /// The log the bus records into (tests read it back).
    pub fn log_for_tests(&self) -> &SqliteEventLogStore {
        &self.log
    }

    /// The `External` commands, sorted — each runs outside the bus's
    /// transaction against a system it doesn't own, so the list is pinned
    /// by a test and grows only on purpose.
    pub fn external_commands(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .read()
            .commands
            .values()
            .filter(|c| matches!(c.handler, Handler::External(_)))
            .map(|c| c.spec.name.clone())
            .collect();
        names.sort();
        names
    }

    /// The `Dispatch` commands (composites included), sorted — each
    /// decides per input whether it runs in the transaction or against a
    /// system outside it, so the list is pinned like
    /// [`Self::external_commands`].
    pub fn dispatch_commands(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .read()
            .commands
            .values()
            .filter(|c| matches!(c.handler, Handler::Dispatch(_) | Handler::Compose(_)))
            .map(|c| c.spec.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Run `name` with `input` as `actor`. `confirmed` says a person has
    /// already confirmed this exact call; an agent's `confirmed` is
    /// ignored — an agent can never confirm.
    pub async fn run(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        self.run_inner(actor, name, input, confirmed, RunOrigin::Call)
            .await
    }

    /// [`Self::run`] as `origin`: the undo of an audit row, or a person's
    /// approval of a proposal. The row is marked (undone; approved) in the
    /// same transaction as the run (a `Tx` handler) or claimed before it
    /// (an `External` one, released if the run fails), so two undos of one
    /// row — or two approvals of one proposal — can't both run.
    async fn run_inner(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
        confirmed: bool,
        origin: RunOrigin,
    ) -> Result<CommandOutcome, CommandError> {
        let command =
            self.commands
                .read()
                .get(name)
                .cloned()
                .ok_or_else(|| CommandError::Unknown {
                    name: name.to_string(),
                })?;
        let spec = &command.spec;

        // 1. The input must match the schema — and, for a `Dispatch`
        // command, name a route.
        let routing = match command
            .validator
            .check(&input)
            .and_then(|()| command.resolve(&input))
        {
            Ok(routing) => routing,
            Err(err) => {
                self.audit_only(actor, spec, &input, Outcome::Invalid, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        };
        // 2. The invoker must be admitted…
        if !spec.invokers.allows(actor.invoker()) {
            let err = CommandError::Denied {
                reason: format!(
                    "`{}` is not open to {:?} callers",
                    spec.name,
                    actor.invoker()
                ),
            };
            self.audit_only(actor, spec, &input, Outcome::Denied, Some(err.to_string()))
                .await;
            return Err(err);
        }
        // 3. …and an agent (or a lens acting for one) must also pass the
        // agent policy.
        // A `Record` command isn't refused here; its handler refuses a
        // claim (see `TxCtx::may_claim`).
        let mut gates = Gates {
            may_claim: true,
            may_write: None,
        };
        if let Some(thread_id) = actor.agent_thread() {
            use oxplow_domain::CommandEffect;
            let may_write = match (spec.effect, &thread_id, &self.write_gate) {
                (CommandEffect::Write | CommandEffect::Record, Some(t), Some(gate)) => {
                    Some(gate(*t).await)
                }
                _ => None,
            };
            gates.may_write = may_write;
            if spec.effect == CommandEffect::Record {
                gates.may_claim = may_write != Some(false);
            }
            let gated = may_write.filter(|_| spec.effect == CommandEffect::Write);
            if let PolicyDecision::Deny { reason, .. } =
                self.policy.check_command(thread_id.as_ref(), spec, gated)
            {
                let err = CommandError::Denied { reason };
                self.audit_only(actor, spec, &input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        }
        // A composite is routed now that the actor is admitted: composed on
        // a read snapshot, each call checked and routed (`steps.rs`).
        let resolved = match routing {
            Routing::Ready(resolved) => resolved,
            Routing::Composite(compose) => match self.route_composite(&compose, &input).await {
                Ok(resolved) => resolved,
                Err(err) => {
                    let recorded = match err {
                        CommandError::Denied { .. } => Outcome::Denied,
                        _ => Outcome::Error,
                    };
                    self.audit_only(actor, spec, &input, recorded, Some(err.to_string()))
                        .await;
                    return Err(err);
                }
            },
        };
        // 4. A person confirms; an agent never can. Nothing is written: a
        // person is asked, an agent's run is kept as a proposal.
        let confirmed = confirmed && !actor.is_agent_driven();
        let confirm = command.confirm(&input);
        if confirm.required() && !confirmed {
            let preview = Preview {
                command: spec.name.clone(),
                summary: spec.summary.clone(),
                input: input.clone(),
                destructive: matches!(confirm, oxplow_domain::Confirm::Destructive),
            };
            let run = Prepared {
                command: &command,
                resolved: &resolved,
            };
            return Err(self
                .unconfirmed(actor, origin, run, input, preview, gates)
                .await);
        }
        // 5a. A read runs without a record: no audit row, no event.
        if spec.effect == oxplow_domain::CommandEffect::Read {
            return self.run_read(&resolved, actor, input).await;
        }
        // 5'. A composite's steps run outside one transaction: each lands
        // as it runs, the run recorded once at the end (`steps.rs`). A step
        // that asks is step 4's answer, as below.
        if let Resolved::Steps(plan) = &resolved {
            let admitted = steps::Admitted {
                confirmed,
                gates,
                origin,
            };
            return match self
                .run_steps(actor, spec, &input, plan.clone(), admitted)
                .await
            {
                Err(CommandError::NeedsConfirmation { preview }) => {
                    let run = Prepared {
                        command: &command,
                        resolved: &resolved,
                    };
                    Err(self
                        .unconfirmed(actor, origin, run, input, *preview, gates)
                        .await)
                }
                other => other,
            };
        }
        // 5. Run, and record the run with its event in one transaction.
        let outcome = match &resolved {
            Resolved::Tx(handler) => {
                let handler = handler.clone();
                let actor_c = actor.clone();
                let spec_c = spec.clone();
                let input_c = input.clone();
                let schemas = self.log.schemas().clone();
                // A handler error must roll the transaction back, which
                // means returning `Err` from the closure; the structured
                // `CommandError` rides out through this slot.
                let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
                let failed_c = failed.clone();
                // An undo or approval that lost a race: not a failed run,
                // so it leaves no audit row.
                let lost: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
                let lost_c = lost.clone();
                let ran = self
                    .db
                    .transaction(move |tx| {
                        let executed_id = oxplow_domain::EventId::generate();
                        let ctx = TxCtx {
                            conn: tx,
                            actor: &actor_c,
                            events: oxplow_db::EventCtx {
                                schemas: &schemas,
                                source: actor_c.source(),
                                cause: Some(executed_id.clone()),
                            },
                            may_claim: gates.may_claim,
                            confirmed,
                            may_write: gates.may_write,
                            depth: 0,
                        };
                        let out = match handler(&ctx, input_c.clone()) {
                            Ok(out) => out,
                            // A lock blip retries the whole run.
                            Err(CommandError::Busy { message }) => {
                                return Err(oxplow_domain::DomainError::Busy(message));
                            }
                            Err(err) => {
                                *failed_c.lock() = Some(err);
                                return Err(oxplow_domain::DomainError::Invariant(
                                    "command handler failed; rolled back".into(),
                                ));
                            }
                        };
                        let recorded = record_tx(
                            tx,
                            &schemas,
                            &actor_c,
                            &spec_c,
                            &input_c,
                            &out,
                            Executed::ok(executed_id),
                        )?;
                        // Fails (and rolls the whole run back) when the row
                        // was undone, or the proposal decided, meanwhile.
                        let marked = match origin {
                            RunOrigin::Call => Ok(()),
                            RunOrigin::Undo(original) => {
                                mark_undone_tx(tx, original, recorded.audit_id)
                            }
                            RunOrigin::Approval(id) => {
                                proposal_store::approve_tx(tx, id, recorded.audit_id).map(|_| ())
                            }
                        };
                        if let Err(e) = marked {
                            let e = lost_race(origin, e)?;
                            *lost_c.lock() = Some(e);
                            return Err(oxplow_domain::DomainError::Invariant(
                                "decided meanwhile; rolled back".into(),
                            ));
                        }
                        if let RunOrigin::Approval(id) = origin {
                            log_approved_tx(tx, &schemas, &actor_c, &spec_c, id, &recorded)?;
                        }
                        Ok((out, recorded))
                    })
                    .await;
                match ran {
                    Ok((out, recorded)) => Ok(finish(out, recorded)),
                    Err(db_err) => {
                        if let Some(e) = lost.lock().take() {
                            return Err(e);
                        }
                        Err(failed
                            .lock()
                            .take()
                            .unwrap_or_else(|| CommandError::from(db_err)))
                    }
                }
            }
            Resolved::Steps(_) => unreachable!("composite steps ran above"),
            Resolved::External(handler) => {
                self.claim(origin).await?;
                match handler(actor.clone(), input.clone()).await {
                    Ok(out) => Ok(self.record_external(actor, spec, &input, out, origin).await),
                    Err(err) => {
                        self.release(origin).await;
                        Err(err)
                    }
                }
            }
        };
        match outcome {
            Ok(done) => {
                self.pump.wake();
                Ok(done)
            }
            // A confirmation a handler raised while running (a composite
            // whose child asks) is step 4's answer, late: rolled back,
            // nothing audited — a person is asked, an agent's run proposed.
            Err(CommandError::NeedsConfirmation { preview }) => {
                let run = Prepared {
                    command: &command,
                    resolved: &resolved,
                };
                Err(self
                    .unconfirmed(actor, origin, run, input, *preview, gates)
                    .await)
            }
            Err(err) => {
                // A handler's refusal (a claim the actor may not take) is a
                // denial, not an error.
                let recorded = match err {
                    CommandError::Denied { .. } => Outcome::Denied,
                    _ => Outcome::Error,
                };
                self.audit_only(actor, spec, &input, recorded, Some(err.to_string()))
                    .await;
                Err(err)
            }
        }
    }

    /// Apply the inverse recorded for `audit_id`, as `actor`, through the
    /// normal pipeline (policy, confirmation and audit included). The
    /// original row is marked `undone_by` the new run.
    pub async fn undo(
        &self,
        actor: &Actor,
        audit_id: i64,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        let row = self
            .audit
            .get(audit_id)
            .await?
            .ok_or_else(|| CommandError::Failed {
                message: format!("audit row {audit_id} not found"),
            })?;
        let inverse = undoable(&row)?;
        self.run_inner(
            actor,
            &inverse.name,
            inverse.input.clone(),
            confirmed,
            RunOrigin::Undo(audit_id),
        )
        .await
    }

    /// A run that needs a confirmation it doesn't have. A person (or the
    /// system) is asked: `NeedsConfirmation`. An agent's run is kept for a
    /// person instead: dry-run (a `Tx` handler, confirmed, in a rolled-back
    /// transaction — what it would have done; an `External` one never
    /// runs), then the proposal and `command.proposed` in one transaction,
    /// and `Proposed`. A dry run that fails is the run's failure, audited
    /// like one. No audit row for a proposal: nothing ran.
    async fn unconfirmed(
        &self,
        actor: &Actor,
        origin: RunOrigin,
        run: Prepared<'_>,
        input: Value,
        preview: Preview,
        gates: Gates,
    ) -> CommandError {
        let Prepared { command, resolved } = run;
        if !actor.is_agent_driven() {
            return CommandError::NeedsConfirmation {
                preview: Box::new(preview),
            };
        }
        // A proposal is a plain call; an undo kept as one would lose the
        // row it undoes (an approval is a person's, so never here).
        if !matches!(origin, RunOrigin::Call) {
            return CommandError::Denied {
                reason: format!(
                    "`{}` needs a person's confirmation; ask them to undo it",
                    command.spec.name
                ),
            };
        }
        let spec = &command.spec;
        let dry_run = match resolved {
            Resolved::Tx(handler) => {
                match self.dry_run(handler.clone(), actor, &input, gates).await {
                    Ok(result) => Some(result),
                    Err(err) => {
                        let recorded = match err {
                            CommandError::Denied { .. } => Outcome::Denied,
                            _ => Outcome::Error,
                        };
                        self.audit_only(actor, spec, &input, recorded, Some(err.to_string()))
                            .await;
                        return err;
                    }
                }
            }
            // Nothing outside the transaction runs before a person decides.
            Resolved::External(_) | Resolved::Steps(_) => None,
        };
        let row = NewProposal {
            command: spec.name.clone(),
            input,
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            stream_id: actor.anchors().stream_id,
            preview: serde_json::to_value(&preview).expect("a preview serializes"),
            dry_run,
        };
        let (actor_c, schemas, destructive) = (
            actor.clone(),
            self.log.schemas().clone(),
            preview.destructive,
        );
        let stored = self
            .db
            .transaction(move |tx| {
                let inserted = proposal_store::insert_tx(tx, &row)?;
                let supersedes: Vec<String> = inserted
                    .superseded
                    .iter()
                    .map(|id| proposal_ref(*id))
                    .collect();
                let proposed = Envelope::typed::<CommandProposed>(
                    actor_c.source(),
                    &CommandProposedV1 {
                        proposal: proposal_ref(inserted.id),
                        command: row.command.clone(),
                        actor_kind: actor_c.kind(),
                        actor_id: actor_c.id(),
                        destructive,
                        supersedes: supersedes.clone(),
                    },
                )
                .with_anchors(actor_c.anchors())
                .with_subject([proposal_ref(inserted.id), command_ref(&row.command)]);
                append_tx(tx, &schemas, &proposed)?;
                Ok((inserted.id, supersedes))
            })
            .await;
        match stored {
            Ok((id, supersedes)) => {
                self.pump.wake();
                CommandError::Proposed {
                    proposal: proposal_ref(id),
                    preview: Box::new(preview),
                    supersedes,
                }
            }
            Err(e) => CommandError::from(e),
        }
    }

    /// A `Tx` handler's result, run confirmed in a transaction that is
    /// rolled back: what it would do now. Its events and `after_commit`
    /// are dropped with it.
    async fn dry_run(
        &self,
        handler: Arc<TxHandler>,
        actor: &Actor,
        input: &Value,
        gates: Gates,
    ) -> Result<Value, CommandError> {
        let (actor, input) = (actor.clone(), input.clone());
        let schemas = self.log.schemas().clone();
        let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
        let failed_c = failed.clone();
        self.db
            .rehearse(move |tx| {
                let ctx = TxCtx {
                    conn: tx,
                    actor: &actor,
                    events: oxplow_db::EventCtx {
                        schemas: &schemas,
                        source: actor.source(),
                        cause: None,
                    },
                    may_claim: gates.may_claim,
                    confirmed: true,
                    may_write: gates.may_write,
                    depth: 0,
                };
                match handler(&ctx, input.clone()) {
                    Ok(out) => Ok(out.result),
                    Err(CommandError::Busy { message }) => {
                        Err(oxplow_domain::DomainError::Busy(message))
                    }
                    Err(err) => {
                        *failed_c.lock() = Some(err);
                        Err(oxplow_domain::DomainError::Invariant(
                            "dry run failed".into(),
                        ))
                    }
                }
            })
            .await
            .map_err(|db_err| {
                failed
                    .lock()
                    .take()
                    .unwrap_or_else(|| CommandError::from(db_err))
            })
    }

    /// A person approves proposal `id`: its command runs as them,
    /// confirmed, through the whole pipeline, and the proposal is marked
    /// approved with the run's audit row (and `command.approved` logged) in
    /// the run's own transaction. A run that fails leaves it pending.
    /// Approving is a person's only.
    pub async fn approve(&self, actor: &Actor, id: i64) -> Result<CommandOutcome, CommandError> {
        let proposal = self.pending_proposal(actor, id).await?;
        self.run_inner(
            actor,
            &proposal.command,
            proposal.input,
            true,
            RunOrigin::Approval(id),
        )
        .await
    }

    /// A person declines proposal `id`: the decision and
    /// `command.declined`, nothing run. A person's only.
    pub async fn decline(&self, actor: &Actor, id: i64) -> Result<(), CommandError> {
        self.pending_proposal(actor, id).await?;
        let (actor_c, schemas) = (actor.clone(), self.log.schemas().clone());
        self.db
            .transaction(move |tx| {
                let declined = proposal_store::decline_tx(tx, id)?;
                let event = Envelope::typed::<CommandDeclined>(
                    actor_c.source(),
                    &CommandDeclinedV1 {
                        proposal: proposal_ref(id),
                        command: declined.command.clone(),
                    },
                )
                .with_subject([proposal_ref(id), command_ref(&declined.command)]);
                append_tx(tx, &schemas, &event)?;
                Ok(())
            })
            .await?;
        self.pump.wake();
        Ok(())
    }

    /// Proposal `id`, when `actor` is a person and it still waits.
    async fn pending_proposal(
        &self,
        actor: &Actor,
        id: i64,
    ) -> Result<oxplow_db::Proposal, CommandError> {
        if !matches!(actor, Actor::Human) {
            return Err(CommandError::Denied {
                reason: "only a person approves or declines a proposal".into(),
            });
        }
        let proposal = self
            .proposals
            .get(id)
            .await?
            .ok_or_else(|| CommandError::Invalid {
                field: Some("/proposal".into()),
                message: format!("{} does not exist", proposal_ref(id)),
            })?;
        if proposal.decision != ProposalDecision::Pending {
            return Err(CommandError::Invalid {
                field: Some("/proposal".into()),
                message: format!(
                    "{} is already {}",
                    proposal_ref(id),
                    proposal.decision.as_str()
                ),
            });
        }
        Ok(proposal)
    }

    /// Run `calls` as one: the children of a composite (`command.sequence`,
    /// an extension's command) inside the parent's transaction and audit
    /// row (P6b.A1). First a pass that writes nothing — every call must
    /// name a `Tx` command (an `External` one can't join the transaction),
    /// its input must fit, and its own `invokers`, the agent policy and
    /// its `confirm` apply, so a composite never widens what its children
    /// allow; a child that asks makes the parent ask, unless the run was
    /// confirmed — then every handler runs on `ctx`, one level deeper. The
    /// children's events ride out on the parent's; the inverse is the
    /// children's inverses, reversed, as a `command.sequence`, or none when
    /// a child has none — so a composite declared undoable whose child
    /// isn't (or gave no inverse) is recorded with no inverse, and its
    /// undo is refused as "not undoable" rather than half-applied.
    pub fn run_nested(
        &self,
        ctx: &TxCtx<'_>,
        parent: &CommandSpec,
        calls: &[CommandCall],
    ) -> Result<NestedOutcome, CommandError> {
        if ctx.depth >= MAX_NESTING {
            return Err(CommandError::Invalid {
                field: None,
                message: format!(
                    "`{}` is nested more than {MAX_NESTING} composites deep — does a command \
                     compose itself?",
                    parent.name
                ),
            });
        }
        let resolved: Vec<Arc<Command>> = {
            let registry = self.commands.read();
            calls
                .iter()
                .map(|call| {
                    registry
                        .get(&call.name)
                        .cloned()
                        .ok_or_else(|| CommandError::Unknown {
                            name: call.name.clone(),
                        })
                })
                .collect::<Result<_, _>>()?
        };
        let mut asks = false;
        let mut destructive = false;
        let mut handlers: Vec<Arc<TxHandler>> = Vec::with_capacity(calls.len());
        for (i, (call, command)) in calls.iter().zip(&resolved).enumerate() {
            let spec = &command.spec;
            let name_field = || Some(format!("/calls/{i}/name"));
            let at_input = |e: CommandError| match e {
                CommandError::Invalid { field, message } => CommandError::Invalid {
                    field: Some(format!("/calls/{i}/input{}", field.unwrap_or_default())),
                    message,
                },
                other => other,
            };
            // This composite runs in a transaction (the bus routed every
            // call inside one, or it is one step of a composite's steps):
            // a call that leaves it can't join.
            let external = |what: String| CommandError::Invalid {
                field: name_field(),
                message: format!(
                    "`{}` {what}, outside the transaction `{}` runs in",
                    spec.name, parent.name
                ),
            };
            if spec.effect == oxplow_domain::CommandEffect::Read {
                return Err(CommandError::Invalid {
                    field: name_field(),
                    message: format!(
                        "`{}` only reads; a composite composes commands that write",
                        spec.name
                    ),
                });
            }
            command.validator.check(&call.input).map_err(at_input)?;
            let handler = match &command.handler {
                Handler::Tx(h) => h.clone(),
                Handler::External(_) => return Err(external("runs against a system".into())),
                Handler::Dispatch(d) => match (d.route)(&call.input).map_err(at_input)? {
                    Route::Tx => d.tx.clone(),
                    Route::External(system) => {
                        return Err(external(format!("runs through {system} for this input")));
                    }
                },
                // Its own calls join this transaction too, or are refused
                // there.
                Handler::Compose(c) => c.tx.clone(),
            };
            handlers.push(handler);
            if !spec.invokers.allows(ctx.actor.invoker()) {
                return Err(CommandError::Denied {
                    reason: format!(
                        "`{}` is not open to {:?} callers",
                        spec.name,
                        ctx.actor.invoker()
                    ),
                });
            }
            if let Some(thread_id) = ctx.actor.agent_thread() {
                let gated = ctx
                    .may_write
                    .filter(|_| spec.effect == oxplow_domain::CommandEffect::Write);
                if let PolicyDecision::Deny { reason, .. } =
                    self.policy.check_command(thread_id.as_ref(), spec, gated)
                {
                    return Err(CommandError::Denied { reason });
                }
            }
            let confirm = command.confirm(&call.input);
            if confirm.required() {
                asks = true;
                destructive |= matches!(confirm, oxplow_domain::Confirm::Destructive);
            }
        }
        if asks && !ctx.confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: parent.name.clone(),
                    summary: parent.summary.clone(),
                    input: serde_json::json!({ "calls": calls }),
                    destructive,
                }),
            });
        }
        let inner = TxCtx {
            conn: ctx.conn,
            actor: ctx.actor,
            events: ctx.events.clone(),
            may_claim: ctx.may_claim,
            confirmed: ctx.confirmed,
            may_write: ctx.may_write,
            depth: ctx.depth + 1,
        };
        let mut children = Vec::with_capacity(calls.len());
        let mut events = Vec::new();
        let mut afters: Vec<Box<dyn FnOnce() + Send + Sync>> = Vec::new();
        for ((call, command), handler) in calls.iter().zip(&resolved).zip(&handlers) {
            let mut out = handler(&inner, call.input.clone())?;
            if let Some(after) = out.after_commit.take() {
                afters.push(after);
            }
            events.extend(out.events);
            children.push(NestedChild {
                name: call.name.clone(),
                input: call.input.clone(),
                result: out.result,
                inverse: if command.spec.undoable {
                    out.inverse
                } else {
                    None
                },
            });
        }
        let inverse = children
            .iter()
            .map(|c| c.inverse.clone())
            .rev()
            .collect::<Option<Vec<_>>>()
            .map(|calls| CommandCall {
                name: compose::SEQUENCE.into(),
                input: serde_json::json!({ "calls": calls }),
            });
        let after_commit: Option<Box<dyn FnOnce() + Send + Sync>> = if afters.is_empty() {
            None
        } else {
            Some(Box::new(move || {
                for after in afters {
                    after();
                }
            }))
        };
        Ok(NestedOutcome {
            children,
            inverse,
            events,
            after_commit,
        })
    }

    /// Step 5 for a `Read` command: the handler on a plain connection,
    /// nothing recorded. A `Read` must not write (it isn't audited).
    async fn run_read(
        &self,
        resolved: &Resolved,
        actor: &Actor,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        let out = match resolved {
            Resolved::Tx(handler) => {
                let handler = handler.clone();
                let actor = actor.clone();
                let schemas = self.log.schemas().clone();
                let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
                let failed_c = failed.clone();
                // A read snapshot, always rolled back: a Read handler's
                // stray write can't land (it isn't audited).
                self.db
                    .read(move |tx| {
                        let ctx = TxCtx {
                            conn: tx,
                            actor: &actor,
                            events: oxplow_db::EventCtx {
                                schemas: &schemas,
                                source: actor.source(),
                                cause: None,
                            },
                            may_claim: false,
                            confirmed: false,
                            may_write: None,
                            depth: 0,
                        };
                        handler(&ctx, input.clone()).map_err(|err| {
                            *failed_c.lock() = Some(err);
                            oxplow_domain::DomainError::Invariant("read command failed".into())
                        })
                    })
                    .await
                    .map_err(|db_err| {
                        failed
                            .lock()
                            .take()
                            .unwrap_or_else(|| CommandError::from(db_err))
                    })?
            }
            Resolved::External(handler) => handler(actor.clone(), input).await?,
            Resolved::Steps(_) => {
                return Err(CommandError::Invalid {
                    field: None,
                    message: "a command that only reads can't compose steps that write".into(),
                })
            }
        };
        Ok(CommandOutcome {
            result: out.result,
            audit_id: None,
            event_id: None,
            inverse: None,
        })
    }

    /// Audit a run that wrote nothing (invalid input, a denied invoker, a
    /// failed handler). Best effort: an audit failure must not mask the
    /// error being reported.
    async fn audit_only(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        outcome: Outcome,
        error: Option<String>,
    ) {
        let row = NewCommandAudit {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: input.clone(),
            outcome,
            error,
            result: None,
            inverse: None,
        };
        if let Err(e) = self
            .db
            .transaction(move |tx| insert_tx(tx, &row).map(|_| ()))
            .await
        {
            tracing::warn!(error = %e, command = %spec.name, "command audit write failed");
        }
    }

    /// Record an `External` run whose effects already happened: the audit
    /// row and `command.executed` (and, for an undo, the claimed row's
    /// real `undone_by`). If recording fails, the run still happened — it
    /// is reported as done and unrecorded (logged at error level), never
    /// as an error, which would tell the caller the change didn't happen.
    async fn record_external(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        mut out: HandlerOutput,
        origin: RunOrigin,
    ) -> CommandOutcome {
        let (actor_c, spec_c, input_c) = (actor.clone(), spec.clone(), input.clone());
        let schemas = self.log.schemas().clone();
        let shadow = HandlerOutput {
            result: out.result.clone(),
            inverse: out.inverse.clone(),
            events: out.events.clone(),
            after_commit: None,
        };
        let recorded = self
            .db
            .transaction(move |tx| {
                let recorded = record_tx(
                    tx,
                    &schemas,
                    &actor_c,
                    &spec_c,
                    &input_c,
                    &shadow,
                    Executed::ok(oxplow_domain::EventId::generate()),
                )?;
                match origin {
                    RunOrigin::Call => {}
                    RunOrigin::Undo(original) => {
                        finish_undo_claim_tx(tx, original, recorded.audit_id)?;
                    }
                    RunOrigin::Approval(id) => {
                        proposal_store::finish_claim_tx(tx, id, recorded.audit_id)?;
                        log_approved_tx(tx, &schemas, &actor_c, &spec_c, id, &recorded)?;
                    }
                }
                Ok(recorded)
            })
            .await;
        match recorded {
            Ok(recorded) => finish(out, recorded),
            Err(e) => {
                tracing::error!(
                    command = %spec.name,
                    error = %e,
                    "command ran but recording it failed; the change stands unrecorded"
                );
                if let Some(after) = out.after_commit.take() {
                    after();
                }
                CommandOutcome {
                    result: out.result,
                    audit_id: None,
                    event_id: None,
                    inverse: None,
                }
            }
        }
    }

    /// Before an `External` run with an origin: mark the audit row as
    /// being undone (`undone_by = UNDO_PENDING`), or the proposal as being
    /// approved (approved, no audit row yet). Fails when it's already
    /// undone or decided, or being so.
    async fn claim(&self, origin: RunOrigin) -> Result<(), CommandError> {
        match origin {
            RunOrigin::Call => Ok(()),
            RunOrigin::Undo(audit_id) => self
                .db
                .transaction(move |tx| mark_undone_tx(tx, audit_id, UNDO_PENDING))
                .await
                .map_err(|e| lost_race(origin, e).unwrap_or_else(CommandError::from)),
            RunOrigin::Approval(id) => self
                .db
                .transaction(move |tx| proposal_store::claim_tx(tx, id))
                .await
                .map_err(|e| lost_race(origin, e).unwrap_or_else(CommandError::from)),
        }
    }

    /// The `External` run failed: the row is undoable again, the proposal
    /// pending again.
    async fn release(&self, origin: RunOrigin) {
        let released = match origin {
            RunOrigin::Call => return,
            RunOrigin::Undo(audit_id) => {
                self.db
                    .transaction(move |tx| {
                        tx.execute(
                            "UPDATE command_audit SET undone_by = NULL
                              WHERE id = ?1 AND undone_by = ?2",
                            rusqlite::params![audit_id, UNDO_PENDING],
                        )
                        .map(|_| ())
                        .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                    })
                    .await
            }
            RunOrigin::Approval(id) => {
                self.db
                    .transaction(move |tx| proposal_store::release_claim_tx(tx, id))
                    .await
            }
        };
        if let Err(e) = released {
            tracing::error!(?origin, error = %e, "releasing a claim failed");
        }
    }
}

/// Marking an undo or an approval failed with `e`: when the row was
/// undone, or the proposal decided, by a concurrent run (an `Invariant` /
/// `Invalid` from the store), the `Invalid` this run answers — not a failed
/// run, so it is never audited; anything else (storage) passes through.
fn lost_race(
    origin: RunOrigin,
    e: oxplow_domain::DomainError,
) -> Result<CommandError, oxplow_domain::DomainError> {
    use oxplow_domain::DomainError as D;
    if !matches!(e, D::Invariant(_) | D::Invalid(_)) {
        return Err(e);
    }
    let message = match origin {
        RunOrigin::Undo(audit_id) => format!("audit row {audit_id} was already undone"),
        RunOrigin::Approval(id) => format!("proposal:{id} was already decided"),
        RunOrigin::Call => return Err(e),
    };
    Ok(CommandError::Invalid {
        field: None,
        message,
    })
}

/// `undone_by` while an `External` undo is running (audit ids start at 1).
const UNDO_PENDING: i64 = 0;

/// Replace a pending undo claim with the undo run's audit row.
fn finish_undo_claim_tx(
    tx: &rusqlite::Connection,
    original: i64,
    done_by: i64,
) -> Result<(), oxplow_domain::DomainError> {
    tx.execute(
        "UPDATE command_audit SET undone_by = ?2 WHERE id = ?1 AND undone_by = ?3",
        rusqlite::params![original, done_by, UNDO_PENDING],
    )
    .map(|_| ())
    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
}

/// What recording a successful run produced.
struct Recorded {
    audit_id: i64,
    event_id: oxplow_domain::EventId,
}

fn finish(mut out: HandlerOutput, recorded: Recorded) -> CommandOutcome {
    if let Some(after) = out.after_commit.take() {
        after();
    }
    CommandOutcome {
        result: out.result,
        audit_id: Some(recorded.audit_id),
        event_id: Some(recorded.event_id),
        inverse: out.inverse,
    }
}

/// The `command.executed` a run is recorded under: its id (fixed before a
/// `Tx` handler runs, so its events can name it as their cause) and, for
/// a composite's steps that failed partway, why.
pub(super) struct Executed {
    pub id: oxplow_domain::EventId,
    pub failed: Option<String>,
}

impl Executed {
    fn ok(id: oxplow_domain::EventId) -> Self {
        Self { id, failed: None }
    }
}

/// Inside the run's transaction: the audit row, `command.executed`
/// pointing at it, the handler's domain events, and the row's event id.
fn record_tx(
    tx: &rusqlite::Connection,
    schemas: &oxplow_domain::EventSchemaRegistry,
    actor: &Actor,
    spec: &CommandSpec,
    input: &Value,
    out: &HandlerOutput,
    executed: Executed,
) -> Result<Recorded, oxplow_domain::DomainError> {
    let Executed {
        id: executed_id,
        failed,
    } = executed;
    // A run that failed partway (a composite's steps, some landed) is
    // recorded with what landed, as an error.
    let outcome = if failed.is_some() {
        Outcome::Error
    } else {
        Outcome::Ok
    };
    let inverse = if spec.undoable {
        out.inverse.clone()
    } else {
        None
    };
    let audit_id = insert_tx(
        tx,
        &NewCommandAudit {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: input.clone(),
            outcome,
            error: failed,
            result: Some(out.result.clone()),
            inverse: inverse.clone(),
        },
    )?;
    let mut executed = Envelope::typed::<CommandExecuted>(
        actor.source(),
        &CommandExecutedV1 {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            outcome,
            audit_id,
            undoable: inverse.is_some(),
        },
    )
    .with_anchors(actor.anchors())
    .with_subject([command_ref(&spec.name)]);
    executed.id = executed_id;
    append_tx(tx, schemas, &executed)?;
    // A handler's own events carry the actor's thread and stream unless
    // it anchored them itself.
    let fallback = actor.anchors();
    for event in &out.events {
        let mut event = event.clone().with_cause(executed.id.clone());
        event.anchors.thread_id = event.anchors.thread_id.or(fallback.thread_id);
        event.anchors.stream_id = event.anchors.stream_id.or(fallback.stream_id);
        append_tx(tx, schemas, &event)?;
    }
    set_event_id_tx(tx, audit_id, &executed.id)?;
    Ok(Recorded {
        audit_id,
        event_id: executed.id,
    })
}

/// `command.approved` for proposal `id`, caused by the approving run.
fn log_approved_tx(
    tx: &rusqlite::Connection,
    schemas: &oxplow_domain::EventSchemaRegistry,
    actor: &Actor,
    spec: &CommandSpec,
    id: i64,
    recorded: &Recorded,
) -> Result<(), oxplow_domain::DomainError> {
    let approved = Envelope::typed::<CommandApproved>(
        actor.source(),
        &CommandApprovedV1 {
            proposal: proposal_ref(id),
            command: spec.name.clone(),
            audit_id: recorded.audit_id,
        },
    )
    .with_subject([proposal_ref(id), command_ref(&spec.name)])
    .with_cause(recorded.event_id.clone());
    append_tx(tx, schemas, &approved)?;
    Ok(())
}

/// The inverse of an audited run, if it can still be applied.
fn undoable(row: &CommandAudit) -> Result<CommandCall, CommandError> {
    if row.outcome != Outcome::Ok {
        return Err(CommandError::Failed {
            message: format!("audit row {} did not complete; nothing to undo", row.id),
        });
    }
    if let Some(by) = row.undone_by {
        return Err(CommandError::Invalid {
            field: None,
            message: format!("audit row {} was already undone by {by}", row.id),
        });
    }
    row.inverse.clone().ok_or_else(|| CommandError::Failed {
        message: format!("`{}` (audit {}) is not undoable", row.command, row.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{ActorKind, ConfigChanged, ConfigChangedV1};
    use oxplow_domain::{
        CommandEffect, Confirm, EventSchemaRegistry, Invokers, Lifecycle, ThreadId,
    };
    use serde_json::json;

    fn bus() -> (Database, CommandBus) {
        let db = Database::in_memory();
        let log = SqliteEventLogStore::new(db.clone(), Arc::new(EventSchemaRegistry::core()));
        let pump = Arc::new(EventPump::new(db.clone(), log.clone(), vec![]));
        let bus = CommandBus::new(db.clone(), log, Arc::new(AgentPolicy::default()), pump);
        db.clone()
            .transaction(|tx| {
                // The table the kv commands write, and the thread `agent()`
                // runs in (a proposal names it).
                tx.execute_batch(
                    "CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
                     INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                          worktree_path, created_at, updated_at)
                       VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x',
                               '2026-01-01', '2026-01-01');
                     INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                       VALUES (7, 1, 'T', 'active', '2026-01-01', '2026-01-01');",
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .now_or_never_ok();
        (db, bus)
    }

    trait NowOrNever {
        fn now_or_never_ok(self);
    }
    impl<F: Future<Output = Result<(), oxplow_domain::DomainError>>> NowOrNever for F {
        fn now_or_never_ok(self) {
            let rt = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| rt.block_on(self)).unwrap();
        }
    }

    /// A lock blip inside a `Tx` handler (SQLITE_BUSY, a snapshot that
    /// moved under a read-then-write) retries the run like any transaction,
    /// instead of failing the caller; a persistent one surfaces as `Busy`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_busy_tx_handler_is_retried_then_reported_as_busy() {
        let (_db, bus) = bus();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = attempts.clone();
        let flaky = Handler::Tx(Arc::new(move |_ctx: &TxCtx<'_>, input| {
            let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if input["v"] == "always" || n == 0 {
                return Err(CommandError::from(oxplow_domain::DomainError::Busy(
                    "database is locked".into(),
                )));
            }
            Ok(HandlerOutput {
                result: json!({ "attempt": n }),
                inverse: None,
                events: Vec::new(),
                after_commit: None,
            })
        }));
        bus.register(
            Command::new(kv_spec("kv.flaky", Invokers::ALL, Confirm::Never), flaky).unwrap(),
        )
        .unwrap();
        let out = bus
            .run(
                &Actor::Human,
                "kv.flaky",
                json!({"k": "a", "v": "once"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            out.result["attempt"], 1,
            "the second attempt ran and committed"
        );
        let err = bus
            .run(
                &Actor::Human,
                "kv.flaky",
                json!({"k": "a", "v": "always"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Busy { .. }), "{err:?}");
    }

    fn kv_spec(name: &str, invokers: Invokers, confirm: Confirm) -> CommandSpec {
        CommandSpec {
            name: name.into(),
            summary: "Set a key in the test table.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["k", "v"],
                "properties": { "k": { "type": "string" }, "v": { "type": "string" } },
                "additionalProperties": false
            }),
            invokers,
            confirm,
            undoable: true,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
            effect: CommandEffect::Write,
        }
    }

    /// A `Tx` handler: writes `k = v`, returns the inverse (restore the
    /// prior value) and a `config.changed` domain event.
    fn kv_set() -> Handler {
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let conn = ctx.conn;
            let k = input["k"].as_str().unwrap_or_default().to_string();
            let v = input["v"].as_str().unwrap_or_default().to_string();
            if v == "boom" {
                return Err(CommandError::Failed {
                    message: "refused value".into(),
                });
            }
            let before: Option<String> = conn
                .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| r.get(0))
                .ok();
            conn.execute(
                "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [&k, &v],
            )
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
            if v == "half" {
                // A write happened, then the handler fails: it must roll back.
                return Err(CommandError::Failed {
                    message: "failed after writing".into(),
                });
            }
            Ok(HandlerOutput {
                result: json!({ "k": k, "v": v }),
                inverse: Some(CommandCall {
                    name: "kv.set".into(),
                    input: json!({ "k": k, "v": before.clone().unwrap_or_default() }),
                }),
                events: vec![Envelope::typed::<ConfigChanged>(
                    "test",
                    &ConfigChangedV1 {
                        key: k.clone(),
                        before: before.map(Value::String).unwrap_or(Value::Null),
                        after: Value::String(v),
                    },
                )],
                after_commit: None,
            })
        }))
    }

    async fn kv_value(db: &Database, k: &str) -> Option<String> {
        let k = k.to_string();
        db.read(move |tx| {
            Ok(tx
                .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| {
                    r.get::<_, String>(0)
                })
                .ok())
        })
        .await
        .unwrap()
    }

    fn agent() -> Actor {
        Actor::Agent {
            thread_id: Some(ThreadId::new(7)),
            stream_id: None,
        }
    }

    /// A `Read` handler runs in a snapshot that is always rolled back, so
    /// a write it makes (by mistake) never lands — it isn't audited.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_commands_write_never_lands() {
        let (db, bus) = bus();
        let mut spec = kv_spec("kv.sneaky", Invokers::ALL, Confirm::Never);
        spec.effect = CommandEffect::Read;
        bus.register(Command::new(spec, kv_set()).unwrap()).unwrap();
        bus.run(&agent(), "kv.sneaky", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_command_is_not_recorded() {
        let (_db, bus) = bus();
        let mut spec = kv_spec("kv.get", Invokers::ALL, Confirm::Never);
        spec.effect = CommandEffect::Read;
        bus.register(
            Command::new(
                spec,
                Handler::Tx(Arc::new(|_ctx: &TxCtx<'_>, input| {
                    Ok(HandlerOutput {
                        result: input,
                        ..HandlerOutput::default()
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        let out = bus
            .run(&agent(), "kv.get", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        assert_eq!(out.result["k"], "a");
        assert_eq!((out.audit_id, out.event_id), (None, None));
        assert!(bus.log.read_after(0, 10).await.unwrap().is_empty());
        assert!(bus.audit_store().list_recent(10).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_thread_that_may_not_write_is_refused_writes_not_reads() {
        let (db, bus) = bus();
        let bus = bus.with_write_gate(Arc::new(|thread| {
            Box::pin(async move { thread != ThreadId::new(7) })
        }));
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(kv_value(&db, "a").await, None);
        // Another thread may; a person is never gated.
        let other = Actor::Agent {
            thread_id: Some(ThreadId::new(8)),
            stream_id: None,
        };
        bus.run(&other, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        bus.run(&Actor::Human, "kv.set", json!({"k": "b", "v": "1"}), false)
            .await
            .unwrap();
    }

    /// A composite carries step 3's answer to its children: a thread that
    /// may not write can't write through `command.sequence` either.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_thread_that_may_not_write_cant_write_through_a_composite() {
        let (db, bus) = bus();
        let bus = Arc::new(bus.with_write_gate(Arc::new(|thread| {
            Box::pin(async move { thread != ThreadId::new(7) })
        })));
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        // A `Record` composite isn't gated itself (it may only record), so
        // what refuses the run is its `Write` child seeing the answer.
        let mut spec = kv_spec("kv.compose", Invokers::ALL, Confirm::Never);
        spec.effect = CommandEffect::Record;
        spec.input_schema = json!({ "type": "object" });
        let parent = spec.clone();
        let weak = Arc::downgrade(&bus);
        bus.register(
            Command::new(
                spec,
                Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
                    let calls: Vec<CommandCall> =
                        serde_json::from_value(input["calls"].clone()).unwrap();
                    let nested = weak.upgrade().unwrap().run_nested(ctx, &parent, &calls)?;
                    Ok(HandlerOutput {
                        result: json!(null),
                        inverse: nested.inverse,
                        events: nested.events,
                        after_commit: nested.after_commit,
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        let calls = json!({ "calls": [{ "name": "kv.set", "input": { "k": "a", "v": "1" } }] });
        let err = bus
            .run(&agent(), "kv.compose", calls.clone(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(kv_value(&db, "a").await, None);
        // A thread that may write gets through the same composite.
        let other = Actor::Agent {
            thread_id: Some(ThreadId::new(8)),
            stream_id: None,
        };
        bus.run(&other, "kv.compose", calls, false).await.unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tx_command_writes_its_state_audit_and_events_together() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let out = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        assert_eq!(out.result, json!({"k": "a", "v": "1"}));
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        let audit = bus
            .audit_store()
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.command, "kv.set");
        assert_eq!(audit.thread_id, Some(ThreadId::new(7)));
        assert_eq!(audit.outcome, Outcome::Ok);
        assert_eq!(audit.event_id, out.event_id.clone());
        assert_eq!(
            audit.inverse.as_ref().unwrap().input,
            json!({"k": "a", "v": ""})
        );
        // command.executed then the handler's domain event, caused by it,
        // anchored to the actor's thread like the run itself.
        let events = bus.log.read_after(0, 10).await.unwrap();
        assert_eq!(events[1].envelope.anchors.thread_id, Some(ThreadId::new(7)));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].envelope.event_type, "command.executed");
        assert_eq!(
            events[0].envelope.payload["audit_id"],
            out.audit_id.unwrap()
        );
        assert_eq!(events[0].envelope.payload["actor_kind"], "agent");
        assert_eq!(events[0].envelope.source, "agent:thr7");
        assert_eq!(events[0].envelope.subject, vec!["command:kv.set"]);
        assert_eq!(events[1].envelope.event_type, "config.changed");
        assert_eq!(events[1].envelope.cause, out.event_id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_tx_handler_rolls_everything_back_and_is_audited_as_error() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let err = bus
            .run(
                &Actor::Human,
                "kv.set",
                json!({"k": "a", "v": "half"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        assert_eq!(
            kv_value(&db, "a").await,
            None,
            "the handler's write rolled back"
        );
        assert!(bus.log.read_after(0, 10).await.unwrap().is_empty());
        let recent = bus.audit_store().list_recent(5).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].outcome, Outcome::Error);
        assert!(recent[0]
            .error
            .as_deref()
            .unwrap()
            .contains("failed after writing"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn schema_rejection_names_the_field_and_is_audited_invalid() {
        let (_db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let err = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": 3}), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/v"),
            "{err:?}"
        );
        assert_eq!(
            bus.audit_store().list_recent(1).await.unwrap()[0].outcome,
            Outcome::Invalid
        );
        assert!(matches!(
            bus.run(&Actor::Human, "kv.nope", json!({}), false)
                .await
                .unwrap_err(),
            CommandError::Unknown { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invokers_and_agent_policy_deny_before_anything_runs() {
        let (db, bus) = bus();
        bus.register(
            Command::new(
                kv_spec("kv.set", Invokers::HUMAN_ONLY, Confirm::Never),
                kv_set(),
            )
            .unwrap(),
        )
        .unwrap();
        let lens = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        let err = bus
            .run(&lens, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(kv_value(&db, "a").await, None);
        let recent = bus.audit_store().list_recent(5).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|r| r.outcome == Outcome::Denied));
        // `list` shows each actor only what it may run.
        assert!(bus.list(&lens).is_empty());
        assert_eq!(bus.list(&Actor::Human).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn confirmation_is_a_persons_move_and_an_agent_never_gets_past_it() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Always), kv_set()).unwrap(),
        )
        .unwrap();
        // A person: asked first, then confirmed.
        let err = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        let CommandError::NeedsConfirmation { preview } = err else {
            panic!("{err:?}");
        };
        assert_eq!(preview.command, "kv.set");
        assert!(!preview.destructive);
        assert_eq!(kv_value(&db, "a").await, None);
        assert!(
            bus.audit_store().list_recent(5).await.unwrap().is_empty(),
            "nothing audited"
        );
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        // An agent: `confirmed` is ignored; nothing is written — the run
        // waits as a proposal for a person.
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "b", "v": "2"}), true)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        assert_eq!(kv_value(&db, "b").await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn undo_applies_the_inverse_and_marks_the_row_once() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        let second = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "2"}), false)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("2"));
        let undo = bus
            .undo(&Actor::Human, second.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        let row = bus
            .audit_store()
            .get(second.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.undone_by, undo.audit_id);
        let err = bus
            .undo(&Actor::Human, second.audit_id.unwrap(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already undone"), "{err}");
        assert!(bus.undo(&Actor::Human, 9999, false).await.is_err());
    }

    /// Two undos of one row race: exactly one applies the inverse; the
    /// other is refused without running it (an inverse applied twice is a
    /// second, unasked-for change).
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_undos_apply_the_inverse_once() {
        let (db, bus) = bus();
        let bus = Arc::new(bus);
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        let second = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "2"}), false)
            .await
            .unwrap()
            .audit_id
            .unwrap();
        let (b1, b2) = (bus.clone(), bus.clone());
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { b1.undo(&Actor::Human, second, false).await }),
            tokio::spawn(async move { b2.undo(&Actor::Human, second, false).await }),
        );
        let results = [r1.unwrap(), r2.unwrap()];
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        let ok_runs = bus
            .audit_store()
            .list_recent(20)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.outcome == Outcome::Ok)
            .count();
        assert_eq!(ok_runs, 3, "two sets and one undo");
    }

    /// A lens acting for an agent is held to the agent rules: it can't
    /// confirm (its run is proposed), and the agent policy applies.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lens_acting_for_an_agent_is_treated_as_the_agent() {
        let (_db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Always), kv_set()).unwrap(),
        )
        .unwrap();
        let lens = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(agent()),
        };
        let err = bus
            .run(&lens, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        let for_person = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        bus.run(&for_person, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
    }

    /// An `External` handler's effects have happened by the time the bus
    /// records them; if recording then fails, the run still happened —
    /// it's reported as done (unrecorded), not as an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_external_run_whose_record_fails_still_reports_success() {
        let (db, bus) = bus();
        let mut spec = kv_spec("kv.effort", Invokers::ALL, Confirm::Never);
        spec.atomicity = Atomicity::External;
        let writes = db.clone();
        bus.register(
            Command::new(
                spec,
                Handler::External(Arc::new(move |_actor, input| {
                    let db = writes.clone();
                    Box::pin(async move {
                        db.transaction(|tx| {
                            tx.execute("INSERT INTO kv (k, v) VALUES ('e', '1')", [])
                                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
                            Ok(())
                        })
                        .await
                        .map_err(CommandError::from)?;
                        Ok(HandlerOutput {
                            result: input,
                            // An event the log refuses (unregistered type):
                            // recording fails after the write committed.
                            events: vec![
                                Envelope::new("nope.unknown", 1, "test", json!({})).unwrap()
                            ],
                            ..HandlerOutput::default()
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        let out = bus
            .run(
                &Actor::Human,
                "kv.effort",
                json!({"k": "e", "v": "1"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.audit_id, None);
        assert_eq!(kv_value(&db, "e").await.as_deref(), Some("1"));
        let rows = bus.audit_store().list_recent(10).await.unwrap();
        assert!(rows.iter().all(|r| r.outcome != Outcome::Error), "{rows:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn registration_checks_names_atomicity_and_collisions() {
        let (_db, bus) = bus();
        let mut wrong = kv_spec("kv.set", Invokers::ALL, Confirm::Never);
        wrong.atomicity = Atomicity::External;
        assert!(Command::new(wrong, kv_set()).is_err());
        assert!(Command::new(kv_spec("set", Invokers::ALL, Confirm::Never), kv_set()).is_err());
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        assert!(bus
            .register(
                Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap()
            )
            .is_err());
        assert!(bus.external_commands().is_empty());
        // A read is never asked about: nothing would resolve its proposal.
        let mut asking_read = kv_spec("kv.peek", Invokers::ALL, Confirm::Always);
        asking_read.effect = CommandEffect::Read;
        let err = Command::new(asking_read, kv_set()).err().unwrap();
        assert!(err.to_string().contains("only reads"), "{err}");
        let mut read = kv_spec("kv.peek", Invokers::ALL, Confirm::Never);
        read.effect = CommandEffect::Read;
        let err = Command::new(read, kv_set())
            .unwrap()
            .with_confirm_for(Arc::new(|_| Confirm::Always))
            .err()
            .unwrap();
        assert!(err.to_string().contains("only reads"), "{err}");
    }

    /// An agent's undo or approval that needs a person is refused, not
    /// proposed as a plain call that would lose what it undoes.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_undo_that_asks_is_denied_not_proposed() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Always), kv_set()).unwrap(),
        )
        .unwrap();
        let done = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
        let err = bus
            .undo(&agent(), done.audit_id.unwrap(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err}");
        assert!(pending(&db).await.is_empty());
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    }

    /// A namespace is registered whole or not at all, by one owner; core
    /// commands' namespaces are oxplow's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_namespace_registers_whole_under_one_owner() {
        let (_db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        assert_eq!(bus.namespace_owner("kv").as_deref(), Some("oxplow"));
        assert_eq!(bus.namespace_owner("ext"), None);
        let cmd = |name: &str| {
            Command::new(kv_spec(name, Invokers::ALL, Confirm::Never), kv_set()).unwrap()
        };

        // One command outside the namespace: nothing is registered.
        let err = bus
            .register_namespace("ext", "extension:x", vec![cmd("ext.a"), cmd("other.b")])
            .unwrap_err();
        assert!(err.to_string().contains("other.b"), "{err}");
        assert_eq!(bus.namespace_owner("ext"), None);
        assert!(bus.input_schema("ext.a").is_none());

        bus.register_namespace("ext", "extension:x", vec![cmd("ext.a"), cmd("ext.b")])
            .unwrap();
        assert_eq!(bus.namespace_owner("ext").as_deref(), Some("extension:x"));
        // Taken: by another owner, and by a core namespace.
        let err = bus
            .register_namespace("ext", "provider:ext", vec![cmd("ext.c")])
            .unwrap_err();
        assert!(err.to_string().contains("extension:x"), "{err}");
        let err = bus
            .register_namespace("kv", "extension:kv", vec![cmd("kv.other")])
            .unwrap_err();
        assert!(err.to_string().contains("oxplow"), "{err}");

        assert_eq!(bus.unregister_namespace("ext").len(), 2);
        assert_eq!(bus.namespace_owner("ext"), None);
    }

    // ---- P6b.A1: the bus composes ----

    /// A handler that only writes when the run was confirmed: it sees
    /// step 4's answer in `ctx.confirmed`. A refusal it raises itself is
    /// a confirmation, not an error — rolled back, nothing audited.
    fn kv_set_if_confirmed() -> Handler {
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            if !ctx.confirmed {
                return Err(CommandError::NeedsConfirmation {
                    preview: Box::new(Preview {
                        command: "kv.careful".into(),
                        summary: "asks".into(),
                        input: input.clone(),
                        destructive: false,
                    }),
                });
            }
            ctx.conn
                .execute(
                    "INSERT INTO kv (k, v) VALUES (?1, ?2)",
                    [input["k"].as_str().unwrap(), input["v"].as_str().unwrap()],
                )
                .map_err(|e| CommandError::Failed {
                    message: e.to_string(),
                })?;
            Ok(HandlerOutput::default())
        }))
    }

    async fn audits(db: &Database) -> Vec<CommandAudit> {
        SqliteCommandAuditStore::new(db.clone())
            .list_recent(50)
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tx_handler_sees_whether_the_run_was_confirmed() {
        let (db, bus) = bus();
        bus.register(
            Command::new(
                kv_spec("kv.careful", Invokers::ALL, Confirm::Never),
                kv_set_if_confirmed(),
            )
            .unwrap(),
        )
        .unwrap();
        let err = bus
            .run(
                &Actor::Human,
                "kv.careful",
                json!({"k": "a", "v": "1"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        assert_eq!(kv_value(&db, "a").await, None);
        assert!(
            audits(&db).await.is_empty(),
            "a confirmation is not an error"
        );
        bus.run(
            &Actor::Human,
            "kv.careful",
            json!({"k": "a", "v": "1"}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    }

    /// `command.sequence` composes Tx commands in one run: each child's
    /// own invokers, policy and confirmation apply; the children share the
    /// parent's transaction and audit row; the inverse is the children's
    /// inverses, reversed.
    fn composing_bus() -> (Database, Arc<CommandBus>) {
        let (db, bus) = bus();
        let bus = Arc::new(bus);
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        bus.register(
            Command::new(
                kv_spec("kv.secret", Invokers::HUMAN_ONLY, Confirm::Never),
                kv_set(),
            )
            .unwrap(),
        )
        .unwrap();
        bus.register(
            Command::new(
                kv_spec("kv.danger", Invokers::ALL, Confirm::Destructive),
                kv_set(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut plain = kv_spec("kv.plain", Invokers::ALL, Confirm::Never);
        plain.undoable = false;
        bus.register(Command::new(plain, kv_set()).unwrap())
            .unwrap();
        // A system outside the bus's transaction: it writes `kv` in its own
        // transaction, and refuses the value `fail`.
        let mut ext = kv_spec("kv.external", Invokers::ALL, Confirm::Never);
        ext.atomicity = Atomicity::External;
        let outside = db.clone();
        bus.register(
            Command::new(
                ext,
                Handler::External(Arc::new(move |_actor, input| {
                    let db = outside.clone();
                    Box::pin(async move {
                        let k = input["k"].as_str().unwrap_or_default().to_string();
                        let v = input["v"].as_str().unwrap_or_default().to_string();
                        if v == "fail" {
                            return Err(CommandError::Failed {
                                message: "the system refused it".into(),
                            });
                        }
                        db.transaction(move |tx| {
                            tx.execute(
                                "INSERT INTO kv (k, v) VALUES (?1, ?2)
                                 ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                                [&k, &v],
                            )
                            .map(|_| ())
                            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                        })
                        .await
                        .map_err(CommandError::from)?;
                        Ok(HandlerOutput {
                            result: input,
                            ..HandlerOutput::default()
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        bus.register(super::compose::sequence_command(&bus))
            .unwrap();
        (db, bus)
    }

    fn calls(items: &[(&str, &str, &str)]) -> Value {
        json!({ "calls": items.iter().map(|(n, k, v)| json!({ "name": n, "input": { "k": k, "v": v } })).collect::<Vec<_>>() })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_nested_run_applies_each_childs_own_checks() {
        let (db, bus) = composing_bus();
        let agent = Actor::Agent {
            thread_id: Some(oxplow_domain::ThreadId::new(7)),
            stream_id: None,
        };
        let err = bus
            .run(
                &agent,
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.secret", "b", "2")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(
            kv_value(&db, "a").await,
            None,
            "nothing ran: the pre-pass refused first"
        );

        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                json!({ "calls": [{ "name": "kv.set", "input": { "k": "a", "v": "1" } }, { "name": "kv.set", "input": { "k": "b" } }] }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f.starts_with("/calls/1/input")),
            "{err:?}"
        );
        assert_eq!(kv_value(&db, "a").await, None);

        // A composite whose steps leave the transaction can't be one step
        // of another: its steps would land outside the outer run.
        let inner = calls(&[("kv.set", "a", "1"), ("kv.external", "b", "2")]);
        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                json!({ "calls": [{ "name": "kv.set", "input": { "k": "c", "v": "3" } }, { "name": "command.sequence", "input": inner }] }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("outside the transaction")),
            "{err:?}"
        );
        assert_eq!(kv_value(&db, "a").await, None);
        assert_eq!(kv_value(&db, "c").await, None);
    }

    /// The audit rows of `name`, newest first.
    async fn audits_of(
        db: &Database,
        name: &str,
    ) -> Vec<oxplow_db::command_audit_store::CommandAudit> {
        oxplow_db::SqliteCommandAuditStore::new(db.clone())
            .list_recent(50)
            .await
            .unwrap()
            .into_iter()
            .filter(|a| a.command == name)
            .collect()
    }

    /// P7 review (tsk713): a composite with a step outside the transaction
    /// runs its steps in order, each landing as it runs — one audit row
    /// and one `command.executed` naming them all, and no undo (a system
    /// outside the bus can't be rolled back with it).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_composite_with_an_external_step_runs_its_steps_in_order() {
        let (db, bus) = composing_bus();
        let out = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[
                    ("kv.set", "a", "1"),
                    ("kv.external", "b", "2"),
                    ("kv.set", "c", "3"),
                ]),
                false,
            )
            .await
            .unwrap();
        for (k, v) in [("a", "1"), ("b", "2"), ("c", "3")] {
            assert_eq!(kv_value(&db, k).await.as_deref(), Some(v), "{k}");
        }
        let names: Vec<&str> = out.result["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["kv.set", "kv.external", "kv.set"]);
        assert!(out.inverse.is_none(), "not undoable");
        let rows = audits_of(&db, "command.sequence").await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].outcome, Outcome::Ok);
        assert!(rows[0].inverse.is_none());
        for child in ["kv.set", "kv.external"] {
            assert!(
                audits_of(&db, child).await.is_empty(),
                "{child} has no row of its own"
            );
        }
    }

    /// P7 review (tsk713): a step that fails stops the run; the steps
    /// before it stand, and the one audit row says what landed and what
    /// failed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_step_stops_the_run_and_what_landed_stands() {
        let (db, bus) = composing_bus();
        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[
                    ("kv.set", "a", "1"),
                    ("kv.external", "b", "fail"),
                    ("kv.set", "c", "3"),
                ]),
                false,
            )
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("`kv.external`") && message.contains("`kv.set`"),
            "names the failed step and what landed: {message}"
        );
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"), "it stands");
        assert_eq!(kv_value(&db, "c").await, None, "nothing after the failure");
        let rows = audits_of(&db, "command.sequence").await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].outcome, Outcome::Error);
        let result = rows[0].result.clone().unwrap();
        assert_eq!(result["children"].as_array().unwrap().len(), 1);
        assert_eq!(result["failed"]["name"], "kv.external");
        assert!(rows[0].inverse.is_none());
    }

    /// P7 review (tsk713): every step is checked before any runs — its
    /// input, its invokers, the agent policy — and a step that asks makes
    /// the run ask once; an agent's run is a proposal (with no dry run:
    /// nothing outside the transaction runs before a person decides).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_external_composite_is_checked_and_confirmed_before_any_step_runs() {
        let (db, bus) = composing_bus();
        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                json!({ "calls": [{ "name": "kv.external", "input": { "k": "a", "v": "1" } }, { "name": "kv.set", "input": { "k": "b" } }] }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f.starts_with("/calls/1/input")),
            "{err:?}"
        );
        let err = bus
            .run(
                &agent(),
                "command.sequence",
                calls(&[("kv.external", "a", "1"), ("kv.secret", "b", "2")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(kv_value(&db, "a").await, None, "nothing ran");

        let asks = calls(&[("kv.external", "a", "1"), ("kv.danger", "b", "2")]);
        let err = bus
            .run(&Actor::Human, "command.sequence", asks.clone(), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
            "{err:?}"
        );
        let err = bus
            .run(&agent(), "command.sequence", asks.clone(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        assert_eq!(
            kv_value(&db, "a").await,
            None,
            "nothing ran before a person decided"
        );

        bus.run(&Actor::Human, "command.sequence", asks, true)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
    }

    /// A composite runs commands that write; a read can't join it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_nested_run_refuses_a_read_child() {
        let (db, bus) = composing_bus();
        let mut read = kv_spec("kv.peek", Invokers::ALL, Confirm::Never);
        read.effect = CommandEffect::Read;
        bus.register(Command::new(read, kv_set()).unwrap()).unwrap();
        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.peek", "b", "2")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/calls/1/name" && message.contains("only reads")),
            "{err}"
        );
        assert_eq!(kv_value(&db, "a").await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_nested_run_asks_when_any_child_asks() {
        let (db, bus) = composing_bus();
        let input = calls(&[("kv.set", "a", "1"), ("kv.danger", "b", "2")]);
        let err = bus
            .run(&Actor::Human, "command.sequence", input.clone(), false)
            .await
            .unwrap_err();
        match err {
            CommandError::NeedsConfirmation { preview } => {
                assert!(preview.destructive);
                assert_eq!(preview.command, "command.sequence");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            kv_value(&db, "a").await,
            None,
            "nothing written before the answer"
        );
        assert!(
            audits(&db).await.is_empty(),
            "no audit row for a confirmation"
        );
        bus.run(&Actor::Human, "command.sequence", input, true)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_nested_run_is_one_transaction_with_one_audit_row() {
        let (db, bus) = composing_bus();
        let err = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.set", "b", "half")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        assert_eq!(
            kv_value(&db, "a").await,
            None,
            "the first child rolled back with the second"
        );

        let out = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.set", "b", "2")]),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        assert_eq!(out.result["children"][1]["result"]["v"], "2");
        let rows: Vec<_> = audits(&db)
            .await
            .into_iter()
            .filter(|a| a.outcome == Outcome::Ok)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "one audit row for the parent, none for the children"
        );
        assert_eq!(rows[0].command, "command.sequence");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_nested_runs_inverse_is_the_reversed_children_and_undoes() {
        let (db, bus) = composing_bus();
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "0"}), false)
            .await
            .unwrap();
        let out = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.set", "b", "2")]),
                false,
            )
            .await
            .unwrap();
        let inverse = out.inverse.clone().expect("undoable");
        assert_eq!(inverse.name, "command.sequence");
        assert_eq!(
            inverse.input["calls"],
            json!([
                { "name": "kv.set", "input": { "k": "b", "v": "" } },
                { "name": "kv.set", "input": { "k": "a", "v": "0" } }
            ])
        );
        bus.undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("0"));
        assert_eq!(kv_value(&db, "b").await.as_deref(), Some(""));

        // A child that isn't undoable makes the parent not undoable.
        let out = bus
            .run(
                &Actor::Human,
                "command.sequence",
                calls(&[("kv.set", "c", "3"), ("kv.plain", "d", "4")]),
                false,
            )
            .await
            .unwrap();
        assert!(out.inverse.is_none());
    }

    // P6b.A3: a run an agent needs a person to confirm is kept as a
    // proposal; a person approves (it runs as them) or declines it.

    async fn pending(db: &Database) -> Vec<oxplow_db::Proposal> {
        oxplow_db::SqliteProposalStore::new(db.clone())
            .list_pending()
            .await
            .unwrap()
    }

    async fn proposal(db: &Database, id: i64) -> oxplow_db::Proposal {
        oxplow_db::SqliteProposalStore::new(db.clone())
            .get(id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn logged(bus: &CommandBus) -> Vec<oxplow_domain::events::StoredEvent> {
        bus.log_for_tests().read_after(0, 100).await.unwrap()
    }

    fn confirming_bus() -> (Database, CommandBus) {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Always), kv_set()).unwrap(),
        )
        .unwrap();
        (db, bus)
    }

    /// Propose `kv.set k=v` as the agent; the proposal's id.
    async fn propose(bus: &CommandBus, k: &str, v: &str) -> i64 {
        let err = bus
            .run(&agent(), "kv.set", json!({ "k": k, "v": v }), false)
            .await
            .unwrap_err();
        let CommandError::Proposed { proposal, .. } = err else {
            panic!("{err:?}");
        };
        proposal
            .strip_prefix("proposal:")
            .and_then(|id| id.parse().ok())
            .unwrap()
    }

    /// A newer proposal of the same call replaces the pending one, and
    /// says so: in its event and in the answer the agent gets.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_proposal_that_replaces_another_names_it() {
        let (_db, bus) = confirming_bus();
        let first = propose(&bus, "a", "1").await;
        let err = bus
            .run(&agent(), "kv.set", json!({ "k": "a", "v": "1" }), false)
            .await
            .unwrap_err();
        let CommandError::Proposed { supersedes, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(supersedes, &vec![format!("proposal:{first}")]);
        assert!(
            err.to_string()
                .contains(&format!("replaces proposal:{first}")),
            "{err}"
        );
        let proposed: Vec<_> = logged(&bus)
            .await
            .into_iter()
            .filter(|e| e.envelope.event_type == "command.proposed")
            .collect();
        assert_eq!(proposed[0].envelope.payload.get("supersedes"), None);
        assert_eq!(
            proposed[1].envelope.payload["supersedes"],
            json!([format!("proposal:{first}")])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_run_that_needs_confirmation_becomes_a_proposal() {
        let (db, bus) = confirming_bus();
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap_err();
        let CommandError::Proposed {
            proposal, preview, ..
        } = err
        else {
            panic!("{err:?}");
        };
        assert_eq!(preview.command, "kv.set");
        let rows = pending(&db).await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(proposal, format!("proposal:{}", row.id));
        assert_eq!(row.command, "kv.set");
        assert_eq!(row.actor_kind, ActorKind::Agent);
        assert_eq!(row.thread_id, Some(ThreadId::new(7)));
        assert_eq!(
            row.dry_run,
            Some(json!({"k": "a", "v": "1"})),
            "what it would have done"
        );
        assert_eq!(kv_value(&db, "a").await, None, "the dry run rolled back");
        assert!(audits(&db).await.is_empty(), "a proposal is not a run");
        let events = logged(&bus).await;
        assert_eq!(
            events
                .iter()
                .map(|e| e.envelope.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["command.proposed"],
            "only the proposal is logged"
        );
        assert_eq!(events[0].envelope.payload["proposal"], proposal);
        assert_eq!(events[0].envelope.payload["destructive"], false);
        assert!(events[0].envelope.subject.contains(&proposal));
        // A dry run that fails is the run's failure, not a proposal.
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "boom"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        assert_eq!(pending(&db).await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_composite_an_agent_runs_is_proposed_with_its_children() {
        let (db, bus) = composing_bus();
        let err = bus
            .run(
                &agent(),
                "command.sequence",
                calls(&[("kv.set", "a", "1"), ("kv.danger", "b", "2")]),
                false,
            )
            .await
            .unwrap_err();
        let CommandError::Proposed { preview, .. } = err else {
            panic!("{err:?}");
        };
        assert!(preview.destructive);
        let rows = pending(&db).await;
        let children = rows[0].dry_run.as_ref().unwrap()["children"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(children.len(), 2);
        assert_eq!(children[1]["name"], "kv.danger");
        assert_eq!(kv_value(&db, "a").await, None);
        assert!(audits(&db).await.is_empty());
        // Approving runs the whole composite as the person, once.
        let out = bus.approve(&Actor::Human, rows[0].id).await.unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
        let p = proposal(&db, rows[0].id).await;
        assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
        assert_eq!(p.audit_id, out.audit_id);
        assert_eq!(audits(&db).await.len(), 1, "one row for the composite");
    }

    /// Two people approve one proposal at once: it runs once; the other
    /// approval lost the race — `Invalid`, and no audit row of its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_approvals_run_it_once() {
        let (db, bus) = confirming_bus();
        let bus = Arc::new(bus);
        let id = propose(&bus, "a", "1").await;
        let (b1, b2) = (bus.clone(), bus.clone());
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { b1.approve(&Actor::Human, id).await }),
            tokio::spawn(async move { b2.approve(&Actor::Human, id).await }),
        );
        let results = [r1.unwrap(), r2.unwrap()];
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        let lost = results.into_iter().find_map(Result::err).unwrap();
        assert!(
            matches!(&lost, CommandError::Invalid { message, .. } if message.contains(&format!("proposal:{id}"))),
            "{lost:?}"
        );
        assert_eq!(audits(&db).await.len(), 1, "only the run that won");
        let approved = logged(&bus)
            .await
            .into_iter()
            .filter(|e| e.envelope.event_type == "command.approved")
            .count();
        assert_eq!(approved, 1);
    }

    /// An `External` approval whose run fails releases its claim: the
    /// proposal is pending again and can be approved later.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_external_approval_leaves_the_proposal_pending() {
        let (db, bus) = bus();
        let mut spec = kv_spec("kv.remote", Invokers::ALL, Confirm::Always);
        spec.atomicity = Atomicity::External;
        bus.register(
            Command::new(
                spec,
                Handler::External(Arc::new(|_actor, _input| {
                    Box::pin(async move {
                        Err(CommandError::Failed {
                            message: "the remote said no".into(),
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        bus.run(&agent(), "kv.remote", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        let id = pending(&db).await.remove(0).id;
        let err = bus.approve(&Actor::Human, id).await.unwrap_err();
        assert!(err.to_string().contains("the remote said no"), "{err}");
        let p = proposal(&db, id).await;
        assert_eq!(p.decision, oxplow_db::ProposalDecision::Pending);
        assert_eq!(p.audit_id, None);
    }

    /// A dry run is rolled back with everything it would have caused: its
    /// `after_commit` never runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dry_run_never_runs_after_commit() {
        let (db, bus) = bus();
        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fired_c = fired.clone();
        bus.register(
            Command::new(
                kv_spec("kv.set", Invokers::ALL, Confirm::Always),
                Handler::Tx(Arc::new(move |_ctx: &TxCtx<'_>, input| {
                    let fired = fired_c.clone();
                    Ok(HandlerOutput {
                        result: input,
                        after_commit: Some(Box::new(move || {
                            fired.store(true, std::sync::atomic::Ordering::SeqCst)
                        })),
                        ..HandlerOutput::default()
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        propose(&bus, "a", "1").await;
        assert_eq!(pending(&db).await.len(), 1);
        assert!(!fired.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn approving_runs_it_as_the_person_and_marks_the_proposal_with_the_run() {
        let (db, bus) = confirming_bus();
        let id = propose(&bus, "a", "1").await;
        let out = bus.approve(&Actor::Human, id).await.unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        let audit = out.audit_id.unwrap();
        let rows = audits(&db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, audit);
        assert_eq!(rows[0].actor_kind, ActorKind::Human, "it ran as the person");
        let p = proposal(&db, id).await;
        assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
        assert_eq!(p.audit_id, Some(audit));
        let approved = logged(&bus)
            .await
            .into_iter()
            .find(|e| e.envelope.event_type == "command.approved")
            .expect("approval logged");
        assert_eq!(approved.envelope.payload["audit_id"], audit);
        assert_eq!(approved.envelope.cause, out.event_id, "caused by the run");
        // A proposal is decided once.
        let err = bus.approve(&Actor::Human, id).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("already approved")),
            "{err:?}"
        );
        assert_eq!(audits(&db).await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_approval_whose_run_fails_leaves_the_proposal_pending() {
        let (db, bus) = confirming_bus();
        let id = propose(&bus, "a", "1").await;
        // The world moved: the key now exists and the table refuses it.
        db.transaction(|tx| {
            tx.execute_batch(
                "CREATE TRIGGER kv_locked BEFORE INSERT ON kv BEGIN SELECT RAISE(ABORT, 'locked'); END;",
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
        bus.approve(&Actor::Human, id).await.unwrap_err();
        let p = proposal(&db, id).await;
        assert_eq!(p.decision, oxplow_db::ProposalDecision::Pending);
        assert_eq!(p.audit_id, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_external_command_is_proposed_without_a_dry_run_and_approved_after_it_runs() {
        let (db, bus) = bus();
        let mut spec = kv_spec("kv.remote", Invokers::ALL, Confirm::Always);
        spec.atomicity = Atomicity::External;
        bus.register(
            Command::new(
                spec,
                Handler::External(Arc::new(|_actor, input| {
                    Box::pin(async move {
                        Ok(HandlerOutput {
                            result: input,
                            ..HandlerOutput::default()
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        let err = bus
            .run(&agent(), "kv.remote", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        let row = pending(&db).await.remove(0);
        assert_eq!(row.dry_run, None, "an External handler is never dry-run");
        let out = bus.approve(&Actor::Human, row.id).await.unwrap();
        let p = proposal(&db, row.id).await;
        assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
        assert_eq!(p.audit_id, out.audit_id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declining_writes_only_the_decision() {
        let (db, bus) = confirming_bus();
        let id = propose(&bus, "a", "1").await;
        bus.decline(&Actor::Human, id).await.unwrap();
        assert_eq!(kv_value(&db, "a").await, None);
        assert!(audits(&db).await.is_empty());
        assert_eq!(
            proposal(&db, id).await.decision,
            oxplow_db::ProposalDecision::Declined
        );
        let declined = logged(&bus)
            .await
            .into_iter()
            .find(|e| e.envelope.event_type == "command.declined")
            .expect("decline logged");
        assert_eq!(
            declined.envelope.payload["proposal"],
            format!("proposal:{id}")
        );
        let err = bus.approve(&Actor::Human, id).await.unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_a_person_decides_a_proposal() {
        let (db, bus) = confirming_bus();
        let id = propose(&bus, "a", "1").await;
        let lens_for_agent = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(agent()),
        };
        for actor in [agent(), lens_for_agent, Actor::System] {
            let err = bus.approve(&actor, id).await.unwrap_err();
            assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
            let err = bus.decline(&actor, id).await.unwrap_err();
            assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        }
        assert_eq!(
            proposal(&db, id).await.decision,
            oxplow_db::ProposalDecision::Pending
        );
        assert_eq!(kv_value(&db, "a").await, None);
    }

    /// `kv.put`, a `Dispatch` command: a key starting `far:` routes to an
    /// External handler (the "far" system), any other key to `kv_set`'s
    /// Tx handler.
    fn kv_dispatch(confirm: Confirm) -> Command {
        let mut spec = kv_spec("kv.put", Invokers::ALL, confirm);
        spec.atomicity = Atomicity::Dispatch;
        let Handler::Tx(tx) = kv_set() else {
            unreachable!("kv_set is a Tx handler");
        };
        Command::new(
            spec,
            Handler::Dispatch(Dispatch {
                route: Arc::new(|input: &Value| match input["k"].as_str() {
                    Some(k) if k.starts_with("far:") => Ok(Route::External("far".into())),
                    Some(k) if k.starts_with("nowhere:") => Err(CommandError::Invalid {
                        field: Some("/k".into()),
                        message: format!("no system holds `{k}`"),
                    }),
                    _ => Ok(Route::Tx),
                }),
                tx,
                external: Arc::new(|_actor, input| {
                    Box::pin(async move {
                        Ok(HandlerOutput {
                            result: json!({ "far": input["k"] }),
                            inverse: Some(CommandCall {
                                name: "kv.put".into(),
                                input: json!({ "k": input["k"], "v": "was" }),
                            }),
                            ..HandlerOutput::default()
                        })
                    })
                }),
            }),
        )
        .unwrap()
    }

    /// P7.A1: a `Dispatch` command routes by its input and then runs
    /// exactly as a Tx or an External command: its Tx route commits in the
    /// bus's transaction and composes into it; its External route runs the
    /// external handler, and in a composite makes the composite run as
    /// steps (P7 review, tsk713); a route the input doesn't name is the
    /// caller's error. Composites are pinned with the `Dispatch` commands.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dispatch_command_routes_by_input_and_composes_by_its_route() {
        let (db, bus) = bus();
        let bus = Arc::new(bus);
        bus.register(kv_dispatch(Confirm::Never)).unwrap();
        bus.register(compose::sequence_command(&bus)).unwrap();
        assert_eq!(bus.dispatch_commands(), ["command.sequence", "kv.put"]);
        assert!(bus.external_commands().is_empty());
        let mut wrong = kv_spec("kv.wrong", Invokers::ALL, Confirm::Never);
        wrong.atomicity = Atomicity::Tx;
        let Handler::Dispatch(d) = kv_dispatch(Confirm::Never).handler else {
            unreachable!()
        };
        assert!(Command::new(wrong, Handler::Dispatch(d)).is_err());

        let near = bus
            .run(&Actor::Human, "kv.put", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
        assert!(near.audit_id.is_some());
        let far = bus
            .run(
                &Actor::Human,
                "kv.put",
                json!({"k": "far:a", "v": "1"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(far.result["far"], "far:a");
        assert_eq!(
            kv_value(&db, "far:a").await,
            None,
            "the far system's, not ours"
        );
        assert_eq!(far.inverse.unwrap().name, "kv.put");
        assert_eq!(audits(&db).await.len(), 2);

        let err = bus
            .run(
                &Actor::Human,
                "kv.put",
                json!({"k": "nowhere:a", "v": "1"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/k"),
            "{err:?}"
        );

        bus.run(
            &Actor::Human,
            compose::SEQUENCE,
            json!({ "calls": [{ "name": "kv.put", "input": { "k": "b", "v": "2" } }] }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
        let far = bus
            .run(
                &Actor::Human,
                compose::SEQUENCE,
                json!({ "calls": [{ "name": "kv.put", "input": { "k": "far:b", "v": "2" } }] }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(far.result["children"][0]["result"]["far"], "far:b");
        assert!(far.inverse.is_none(), "steps aren't undoable");
    }

    /// An agent's run of a `Dispatch` command that needs a person is
    /// proposed like any other: dry-run on its Tx route (the proposal
    /// shows what it would do), with no dry run on its External route
    /// (nothing outside the transaction runs before a person decides).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_dispatch_run_is_dry_run_only_on_its_tx_route() {
        let (db, bus) = bus();
        bus.register(kv_dispatch(Confirm::Always)).unwrap();
        let proposed = |err: CommandError| -> i64 {
            let CommandError::Proposed { proposal, .. } = err else {
                panic!("{err:?}");
            };
            proposal
                .strip_prefix("proposal:")
                .and_then(|id| id.parse().ok())
                .unwrap()
        };
        let near = bus
            .run(&agent(), "kv.put", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        let near = proposal(&db, proposed(near)).await;
        assert_eq!(near.dry_run.as_ref().map(|d| &d["v"]), Some(&json!("1")));
        let far = bus
            .run(&agent(), "kv.put", json!({"k": "far:a", "v": "1"}), false)
            .await
            .unwrap_err();
        let far = proposal(&db, proposed(far)).await;
        assert_eq!(far.dry_run, None);
        assert_eq!(kv_value(&db, "a").await, None);
        assert!(audits(&db).await.is_empty());
    }
}
