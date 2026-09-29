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
//! writes, the event and the audit commit together) or `BestEffort` (a
//! pre-existing service call with its own transactions, audited after
//! it returns). `BestEffort` exists only for handlers that predate the
//! bus; [`CommandBus::best_effort_count`] is asserted by a test so the
//! number trends to zero.

pub mod config_commands;
pub mod effort;
pub mod work_item;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use oxplow_db::command_audit_store::{insert_tx, mark_undone_tx, set_event_id_tx, NewCommandAudit};
use oxplow_db::event_log_store::append_tx;
use oxplow_db::{CommandAudit, Database, SqliteCommandAuditStore, SqliteEventLogStore};
use oxplow_domain::events::schema::{
    CommandExecuted, CommandExecutedV1, CommandOutcome as Outcome,
};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec, Envelope,
    InputValidator, Preview,
};
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::RwLock;
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
    /// in the same transaction. A `BestEffort` handler's own transactions
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
}

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

type TxHandler = dyn Fn(&TxCtx<'_>, Value) -> Result<HandlerOutput, CommandError> + Send + Sync;
type BestEffortFuture = Pin<Box<dyn Future<Output = Result<HandlerOutput, CommandError>> + Send>>;
type BestEffortHandler = dyn Fn(Actor, Value) -> BestEffortFuture + Send + Sync;

pub enum Handler {
    /// Runs inside the bus's transaction.
    Tx(Arc<TxHandler>),
    /// A pre-existing service call; the bus audits it after it returns.
    BestEffort(Arc<BestEffortHandler>),
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
            Handler::BestEffort(_) => Atomicity::BestEffort,
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
        let validator = InputValidator::compile(&spec.input_schema)?;
        Ok(Self {
            spec,
            handler,
            validator,
            confirm_for: None,
        })
    }

    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Self {
        self.confirm_for = Some(f);
        self
    }

    fn confirm(&self, input: &Value) -> oxplow_domain::Confirm {
        match &self.confirm_for {
            Some(f) => f(input),
            None => self.spec.confirm,
        }
    }
}

/// May this agent thread write? (`Thread::status.is_writer()` in
/// production.) The bus asks before an agent runs a `Write` command.
pub type WriteGate =
    Arc<dyn Fn(oxplow_domain::ThreadId) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

pub struct CommandBus {
    db: Database,
    log: SqliteEventLogStore,
    audit: SqliteCommandAuditStore,
    policy: Arc<AgentPolicy>,
    pump: Arc<EventPump>,
    commands: RwLock<BTreeMap<String, Arc<Command>>>,
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
            db,
            log,
            policy,
            pump,
            commands: RwLock::new(BTreeMap::new()),
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
        let mut commands = self.commands.write();
        if commands.contains_key(&command.spec.name) {
            return Err(CommandError::Invalid {
                field: Some("/name".into()),
                message: format!("command `{}` is already registered", command.spec.name),
            });
        }
        commands.insert(command.spec.name.clone(), Arc::new(command));
        Ok(())
    }

    pub fn audit_store(&self) -> &SqliteCommandAuditStore {
        &self.audit
    }

    /// The specs `actor` may invoke, by name.
    pub fn list(&self, actor: &Actor) -> Vec<CommandSpec> {
        self.commands
            .read()
            .values()
            .filter(|c| c.spec.invokers.allows(actor.invoker()))
            .map(|c| c.spec.clone())
            .collect()
    }

    pub fn spec(&self, name: &str) -> Option<CommandSpec> {
        self.commands.read().get(name).map(|c| c.spec.clone())
    }

    /// The log the bus records into (tests read it back).
    pub fn log_for_tests(&self) -> &SqliteEventLogStore {
        &self.log
    }

    /// How many registered handlers are `BestEffort`. Trends to zero.
    pub fn best_effort_count(&self) -> usize {
        self.commands
            .read()
            .values()
            .filter(|c| matches!(c.handler, Handler::BestEffort(_)))
            .count()
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
        self.run_inner(actor, name, input, confirmed, None).await
    }

    /// [`Self::run`], optionally as the undo of audit row `undo_of`: the
    /// row is claimed as undone in the same transaction as the run (a
    /// `Tx` handler) or before it (a `BestEffort` one, released if the run
    /// fails), so two undos of one row can't both apply the inverse.
    async fn run_inner(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
        confirmed: bool,
        undo_of: Option<i64>,
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

        // 1. The input must match the schema.
        if let Err(err) = command.validator.check(&input) {
            self.audit_only(actor, spec, &input, Outcome::Invalid, Some(err.to_string()))
                .await;
            return Err(err);
        }
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
        let mut may_claim = true;
        if let Some(thread_id) = actor.agent_thread() {
            use oxplow_domain::CommandEffect;
            let may_write = match (spec.effect, &thread_id, &self.write_gate) {
                (CommandEffect::Write | CommandEffect::Record, Some(t), Some(gate)) => {
                    Some(gate(*t).await)
                }
                _ => None,
            };
            if spec.effect == CommandEffect::Record {
                may_claim = may_write != Some(false);
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
        // 4. A person confirms; an agent never can. Nothing is written.
        let confirmed = confirmed && !actor.is_agent_driven();
        let confirm = command.confirm(&input);
        if confirm.required() && !confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: spec.name.clone(),
                    summary: spec.summary.clone(),
                    input,
                    destructive: matches!(confirm, oxplow_domain::Confirm::Destructive),
                }),
            });
        }
        // 5a. A read runs without a record: no audit row, no event.
        if spec.effect == oxplow_domain::CommandEffect::Read {
            return self.run_read(&command, actor, input).await;
        }
        // 5. Run, and record the run with its event in one transaction.
        let outcome = match &command.handler {
            Handler::Tx(handler) => {
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
                            may_claim,
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
                            executed_id,
                        )?;
                        if let Some(original) = undo_of {
                            // Fails (and rolls the whole run back) when the
                            // row was undone meanwhile.
                            mark_undone_tx(tx, original, recorded.audit_id)?;
                        }
                        Ok((out, recorded))
                    })
                    .await;
                match ran {
                    Ok((out, recorded)) => Ok(finish(out, recorded)),
                    Err(db_err) => Err(failed
                        .lock()
                        .take()
                        .unwrap_or_else(|| CommandError::from(db_err))),
                }
            }
            Handler::BestEffort(handler) => {
                if let Some(original) = undo_of {
                    self.claim_undo(original).await?;
                }
                match handler(actor.clone(), input.clone()).await {
                    Ok(out) => Ok(self
                        .record_best_effort(actor, spec, &input, out, undo_of)
                        .await),
                    Err(err) => {
                        if let Some(original) = undo_of {
                            self.release_undo(original).await;
                        }
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
            Some(audit_id),
        )
        .await
        .map_err(|e| match e {
            // The row was undone by a concurrent undo between our read and
            // our claim: say so, as the up-front check would have.
            CommandError::Failed { message } if message.contains("already undone") => {
                CommandError::Failed {
                    message: format!("audit row {audit_id} was already undone"),
                }
            }
            other => other,
        })
    }

    /// Step 5 for a `Read` command: the handler on a plain connection,
    /// nothing recorded. A `Read` must not write (it isn't audited).
    async fn run_read(
        &self,
        command: &Command,
        actor: &Actor,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        let out = match &command.handler {
            Handler::Tx(handler) => {
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
            Handler::BestEffort(handler) => handler(actor.clone(), input).await?,
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

    /// Record a `BestEffort` run whose writes already committed: the audit
    /// row and `command.executed` (and, for an undo, the claimed row's
    /// real `undone_by`). If recording fails, the run still happened — it
    /// is reported as done and unrecorded (logged at error level), never
    /// as an error, which would tell the caller the change didn't happen.
    async fn record_best_effort(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        mut out: HandlerOutput,
        undo_of: Option<i64>,
    ) -> CommandOutcome {
        let (actor_c, spec_c, input_c) = (actor.clone(), spec.clone(), input.clone());
        let schemas = self.log.schemas().clone();
        let shadow = HandlerOutput {
            result: Value::Null,
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
                    oxplow_domain::EventId::generate(),
                )?;
                if let Some(original) = undo_of {
                    finish_undo_claim_tx(tx, original, recorded.audit_id)?;
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

    /// Mark `audit_id` as being undone (`undone_by = UNDO_PENDING`) before
    /// running a `BestEffort` inverse. Fails when it's already undone or
    /// being undone.
    async fn claim_undo(&self, audit_id: i64) -> Result<(), CommandError> {
        self.db
            .transaction(move |tx| mark_undone_tx(tx, audit_id, UNDO_PENDING))
            .await
            .map_err(CommandError::from)
    }

    /// The inverse failed: the row is undoable again.
    async fn release_undo(&self, audit_id: i64) {
        if let Err(e) = self
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE command_audit SET undone_by = NULL WHERE id = ?1 AND undone_by = ?2",
                    rusqlite::params![audit_id, UNDO_PENDING],
                )
                .map(|_| ())
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
        {
            tracing::error!(audit_id, error = %e, "releasing an undo claim failed");
        }
    }
}

/// `undone_by` while a `BestEffort` undo is running (audit ids start at 1).
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

/// Inside the run's transaction: the audit row, `command.executed`
/// pointing at it, the handler's domain events, and the row's event id.
fn record_tx(
    tx: &rusqlite::Connection,
    schemas: &oxplow_domain::EventSchemaRegistry,
    actor: &Actor,
    spec: &CommandSpec,
    input: &Value,
    out: &HandlerOutput,
    executed_id: oxplow_domain::EventId,
) -> Result<Recorded, oxplow_domain::DomainError> {
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
            outcome: Outcome::Ok,
            error: None,
            inverse: inverse.clone(),
        },
    )?;
    let mut executed = Envelope::typed::<CommandExecuted>(
        actor.source(),
        &CommandExecutedV1 {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            outcome: Outcome::Ok,
            audit_id,
            undoable: inverse.is_some(),
        },
    )
    .with_anchors(actor.anchors())
    .with_subject([oxplow_domain::refs::build::command_ref(&spec.name)]);
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

/// The inverse of an audited run, if it can still be applied.
fn undoable(row: &CommandAudit) -> Result<CommandCall, CommandError> {
    if row.outcome != Outcome::Ok {
        return Err(CommandError::Failed {
            message: format!("audit row {} did not complete; nothing to undo", row.id),
        });
    }
    if let Some(by) = row.undone_by {
        return Err(CommandError::Failed {
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
    use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV1};
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
                tx.execute_batch("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)")
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

    async fn kv(db: &Database, k: &str) -> Option<String> {
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
        assert_eq!(kv(&db, "a").await, None);
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
        assert_eq!(kv(&db, "a").await, None);
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
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
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
        assert_eq!(kv(&db, "a").await, None, "the handler's write rolled back");
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
        assert_eq!(kv(&db, "a").await, None);
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
        assert_eq!(kv(&db, "a").await, None);
        assert!(
            bus.audit_store().list_recent(5).await.unwrap().is_empty(),
            "nothing audited"
        );
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
        // An agent: `confirmed` is ignored; nothing is written.
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "b", "v": "2"}), true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        assert_eq!(kv(&db, "b").await, None);
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
        assert_eq!(kv(&db, "a").await.as_deref(), Some("2"));
        let undo = bus
            .undo(&Actor::Human, second.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
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
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
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
    /// confirm, and the agent policy applies.
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
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        let for_person = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        bus.run(&for_person, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
    }

    /// A `BestEffort` handler's writes are committed by the time the bus
    /// records them; if recording then fails, the run still happened —
    /// it's reported as done (unrecorded), not as an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_best_effort_run_whose_record_fails_still_reports_success() {
        let (db, bus) = bus();
        let mut spec = kv_spec("kv.effort", Invokers::ALL, Confirm::Never);
        spec.atomicity = Atomicity::BestEffort;
        let writes = db.clone();
        bus.register(
            Command::new(
                spec,
                Handler::BestEffort(Arc::new(move |_actor, input| {
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
        assert_eq!(kv(&db, "e").await.as_deref(), Some("1"));
        let rows = bus.audit_store().list_recent(10).await.unwrap();
        assert!(rows.iter().all(|r| r.outcome != Outcome::Error), "{rows:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn registration_checks_names_atomicity_and_collisions() {
        let (_db, bus) = bus();
        let mut wrong = kv_spec("kv.set", Invokers::ALL, Confirm::Never);
        wrong.atomicity = Atomicity::BestEffort;
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
        assert_eq!(bus.best_effort_count(), 0);
    }
}
