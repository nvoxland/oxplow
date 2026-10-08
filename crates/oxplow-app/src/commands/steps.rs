//! A composite's **steps** (P7 review, tsk713): when one of the calls a
//! composite makes leaves the bus's transaction — an `External` command,
//! `work_item.*` on another provider's item — the calls can't be one
//! all-or-nothing run. They run as steps instead:
//!
//! 1. **Checked first.** Every call is routed and checked before any runs
//!    — its input, its invokers, the agent policy — and if any asks for a
//!    confirmation the run asks once (an agent's run becomes a proposal,
//!    with no dry run: nothing outside the transaction runs before a
//!    person decides).
//! 2. **Run in order**, each landing as it runs: a step inside oxplow in
//!    its own transaction, an external one through its system. The first
//!    failure stops the run; what landed before it stands.
//! 3. **Recorded once**: one audit row and one `command.executed` naming
//!    every step that landed (and the one that failed), the steps' events
//!    caused by it. Not undoable: a system outside the bus can't be rolled
//!    back with it.
//!
//! Composites routed entirely inside the transaction never come here:
//! they run as one transaction through `CommandBus::run_nested`.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_domain::events::schema::CommandOutcome as Outcome;
use oxplow_domain::{Actor, CommandCall, CommandError, CommandOutcome, CommandSpec, Preview};
use oxplow_runtime::policy::PolicyDecision;
use serde_json::{json, Value};

use super::compose::Compose;
use super::{
    finish_undo_claim_tx, log_approved_tx, proposal_store, record_tx, Command, CommandBus,
    Executed, ExternalHandler, Gates, Handler, HandlerOutput, NestedChild, Resolved, Route,
    RunOrigin, TxCtx, TxHandler, MAX_NESTING,
};

/// How one step runs.
#[derive(Clone)]
pub(super) enum StepRun {
    /// In its own transaction.
    Tx(Arc<TxHandler>),
    /// Against its system.
    External(Arc<ExternalHandler>),
}

/// One routed call of a composite's steps.
pub(super) struct Step {
    command: Arc<Command>,
    call: CommandCall,
    run: StepRun,
}

/// A composite that runs as steps: each call routed, and the composite's
/// own result.
pub(super) struct Plan {
    steps: Vec<Step>,
    result: Option<Value>,
    /// The composite's own events, recorded with the run.
    events: Vec<oxplow_domain::Envelope>,
    /// The host capabilities composing it called.
    capabilities: BTreeMap<String, u32>,
}

/// Steps 3 and 4's answers for a run, and what it is: what its steps run
/// with.
#[derive(Clone)]
pub(super) struct Admitted {
    pub confirmed: bool,
    pub gates: Gates,
    pub origin: RunOrigin,
}

/// A check to run before the transaction ([`super::Precheck`]): a
/// command's own for its input, or a composed call's, its refusal placed
/// at the call's input (`/calls/<i>/input…`).
pub(super) struct Pending {
    check: Arc<super::Precheck>,
    input: Value,
    at: String,
}

impl Pending {
    /// The command's own check of the input it was called with.
    pub(super) fn own(check: Arc<super::Precheck>, input: Value) -> Self {
        Self {
            check,
            input,
            at: String::new(),
        }
    }

    pub(super) async fn run(self) -> Result<(), CommandError> {
        (self.check)(self.input).await.map_err(|e| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field: Some(format!("{}{}", self.at, field.unwrap_or_default())),
                message,
            },
            other => other,
        })
    }
}

/// Where a composite's calls run.
enum Routed {
    /// Every call joins the transaction.
    Tx,
    /// At least one leaves it.
    Steps(Plan),
}

/// Routes a composite's calls on a read snapshot, collecting each call's
/// check before the transaction (tsk1010).
struct Router<'a> {
    registry: &'a BTreeMap<String, Arc<Command>>,
    conn: &'a rusqlite::Connection,
    prechecks: Vec<Pending>,
}

impl Router<'_> {
    /// Compose `compose` for `input` on the snapshot and route each call; a
    /// call that is itself a composite must route inside the transaction
    /// (its steps would otherwise land outside the run they're a step of).
    /// Each call's check before the transaction is collected, placed at `at`
    /// + the call's input.
    fn route(
        &mut self,
        compose: &Compose,
        input: &Value,
        name: &str,
        depth: usize,
        at: &str,
    ) -> Result<Routed, CommandError> {
        let (registry, conn) = (self.registry, self.conn);
        if depth >= MAX_NESTING {
            return Err(CommandError::Invalid {
                field: None,
                message: format!(
                    "`{name}` is nested more than {MAX_NESTING} composites deep — does a command \
                 compose itself?"
                ),
            });
        }
        // What composing calls is the run's when it runs as steps (this
        // composition is the one that runs); a nested composite composes
        // again in its step's transaction, counted there.
        let trace = crate::host_capabilities::CapabilityTrace::default();
        let composition = (compose.compose)(conn, &trace, input)?;
        let mut steps = Vec::with_capacity(composition.calls.len());
        let mut outside = false;
        for (i, call) in composition.calls.into_iter().enumerate() {
            let name_field = || Some(format!("/calls/{i}/name"));
            let at_input = |e: CommandError| match e {
                CommandError::Invalid { field, message } => CommandError::Invalid {
                    field: Some(format!("/calls/{i}/input{}", field.unwrap_or_default())),
                    message,
                },
                other => other,
            };
            let command =
                registry
                    .get(&call.name)
                    .cloned()
                    .ok_or_else(|| CommandError::Unknown {
                        name: call.name.clone(),
                    })?;
            if command.spec.effect == oxplow_domain::CommandEffect::Read {
                return Err(CommandError::Invalid {
                    field: name_field(),
                    message: format!(
                        "`{}` only reads; a composite composes commands that write",
                        call.name
                    ),
                });
            }
            command.validator.check(&call.input).map_err(at_input)?;
            let call_at = format!("{at}/calls/{i}/input");
            if let Some(check) = &command.precheck {
                self.prechecks.push(Pending {
                    check: check.clone(),
                    input: call.input.clone(),
                    at: call_at.clone(),
                });
            }
            let run = match &command.handler {
                Handler::Tx(h) => StepRun::Tx(h.clone()),
                Handler::External(h) => {
                    outside = true;
                    StepRun::External(h.clone())
                }
                Handler::Dispatch(d) => match (d.route)(&call.input).map_err(at_input)? {
                    Route::Tx => StepRun::Tx(d.tx.clone()),
                    Route::External(_) => {
                        outside = true;
                        StepRun::External(d.external.clone())
                    }
                },
                Handler::Compose(c) => {
                    match self.route(c, &call.input, &call.name, depth + 1, &call_at)? {
                        Routed::Tx => StepRun::Tx(c.tx.clone()),
                        Routed::Steps(_) => {
                            return Err(CommandError::Invalid {
                                field: name_field(),
                                message: format!(
                                    "`{}` runs steps outside the transaction, so it can't be one \
                                 step of another composite",
                                    call.name
                                ),
                            })
                        }
                    }
                }
            };
            steps.push(Step { command, call, run });
        }
        Ok(if outside {
            Routed::Steps(Plan {
                steps,
                result: composition.result,
                events: composition.events,
                capabilities: trace.summary(),
            })
        } else {
            Routed::Tx
        })
    }
}

/// A step sent with an idempotency key is the same write on every
/// attempt, and its system answers a re-send with the same answer, events
/// included: each event without a dedupe key of its own takes one from
/// the key, so a re-sent step that had landed logs its events once
/// (tsk912; `record_tx` skips an event already logged under its key).
fn same_events_once(mut out: HandlerOutput, key: Option<&str>) -> HandlerOutput {
    if let Some(key) = key {
        for (i, event) in out.events.iter_mut().enumerate() {
            if event.dedupe_key.is_none() {
                event.dedupe_key = Some(format!("{key}:event:{i}"));
            }
        }
    }
    out
}

/// A step's failure as the run reports it: the step that failed and the
/// ones that landed before it, which stand — of the failing step's kind
/// (tsk914): `Unavailable` (worth sending again) stays so, with its wait,
/// and anything else is `Failed`.
fn stopped(failed: &str, landed: &[NestedChild], err: &CommandError) -> CommandError {
    let landed: Vec<String> = landed.iter().map(|c| format!("`{}`", c.name)).collect();
    let message = format!(
        "`{failed}` failed after {} ran — they stand, and the run can't be undone: {err}",
        landed.join(", ")
    );
    match err {
        CommandError::Unavailable { retry_after_ms, .. } => CommandError::Unavailable {
            message,
            retry_after_ms: *retry_after_ms,
        },
        _ => CommandError::Failed { message },
    }
}

impl CommandBus {
    /// Route composite `compose` for `input`: composed on a read snapshot,
    /// each call checked and routed. Every call inside the transaction →
    /// its `Tx` handler (which composes again, in the run's own
    /// transaction); any outside → its steps. With it, every composed
    /// call's check before the transaction (tsk1010).
    pub(super) async fn route_composite(
        &self,
        compose: &Arc<Compose>,
        input: &Value,
    ) -> Result<(Resolved, Vec<Pending>), CommandError> {
        let registry = self.commands.read().commands.clone();
        let (c, input) = (compose.clone(), input.clone());
        let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
        let failed_c = failed.clone();
        let routed = self
            .db
            .read(move |conn| {
                let mut router = Router {
                    registry: &registry,
                    conn,
                    prechecks: Vec::new(),
                };
                router
                    .route(&c, &input, "", 0, "")
                    .map(|routed| (routed, router.prechecks))
                    .map_err(|err| {
                        *failed_c.lock() = Some(err);
                        oxplow_domain::DomainError::Invariant("routing a composite failed".into())
                    })
            })
            .await
            .map_err(|db_err| {
                failed
                    .lock()
                    .take()
                    .unwrap_or_else(|| CommandError::from(db_err))
            })?;
        let (routed, prechecks) = routed;
        Ok((
            match routed {
                Routed::Tx => Resolved::Tx(compose.tx.clone()),
                Routed::Steps(plan) => Resolved::Steps(Arc::new(plan)),
            },
            prechecks,
        ))
    }

    /// Run `plan`'s steps as `actor` (see the module doc).
    pub(super) async fn run_steps(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        plan: Arc<Plan>,
        admitted: Admitted,
    ) -> Result<CommandOutcome, CommandError> {
        let Admitted {
            confirmed,
            gates,
            origin,
        } = admitted;
        // 1. Every step's own invokers, policy and confirmation, before
        // any runs.
        let mut asks = false;
        let mut destructive = false;
        for step in &plan.steps {
            let child = &step.command.spec;
            let refused = if !child.invokers.allows(actor.invoker()) {
                Some(format!(
                    "`{}` is not open to {:?} callers",
                    child.id,
                    actor.invoker()
                ))
            } else if let Some(thread_id) = actor.agent_thread() {
                let gated = gates
                    .may_write
                    .filter(|_| child.effect == oxplow_domain::CommandEffect::Write);
                match self.policy.check_command(thread_id.as_ref(), child, gated) {
                    PolicyDecision::Deny { reason, .. } => Some(reason),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(reason) = refused {
                let err = CommandError::Denied { reason };
                self.audit_only(actor, spec, input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
            let confirm = step.command.confirm(&step.call.input);
            asks |= confirm.required();
            destructive |= matches!(confirm, oxplow_domain::Confirm::Destructive);
        }
        let calls: Vec<&CommandCall> = plan.steps.iter().map(|s| &s.call).collect();
        if asks && !confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: spec.id.clone(),
                    summary: spec.summary.clone(),
                    input: json!({ "calls": calls }),
                    destructive,
                }),
            });
        }

        // 2. In order, each landing as it runs; the first failure stops.
        self.claim(&origin).await?;
        let executed_id = oxplow_domain::EventId::generate();
        let mut landed: Vec<NestedChild> = Vec::with_capacity(plan.steps.len());
        let mut events = Vec::new();
        let mut failure: Option<(&Step, CommandError)> = None;
        let mut capabilities = plan.capabilities.clone();
        for (index, step) in plan.steps.iter().enumerate() {
            let ran = match &step.run {
                StepRun::Tx(handler) => {
                    self.run_step_tx(
                        handler.clone(),
                        actor,
                        &step.call.input,
                        &executed_id,
                        confirmed,
                        gates,
                    )
                    .await
                }
                StepRun::External(handler) => {
                    let invocation =
                        origin.invocation(actor, index, &step.call.name, &step.call.input);
                    let key = invocation.idempotency_key.clone();
                    handler(invocation, step.call.input.clone())
                        .await
                        .map(|out| (same_events_once(out, key.as_deref()), BTreeMap::new()))
                }
            };
            match ran {
                Ok((mut out, used)) => {
                    crate::host_capabilities::add_counts(&mut capabilities, used);
                    if let Some(after) = out.after_commit.take() {
                        after();
                    }
                    events.extend(out.events);
                    landed.push(NestedChild {
                        name: step.call.name.clone(),
                        input: step.call.input.clone(),
                        result: out.result,
                        inverse: out.inverse.filter(|_| step.command.spec.undoable),
                    });
                }
                Err(err) => {
                    failure = Some((step, err));
                    break;
                }
            }
        }
        // Nothing landed: the run failed like any other, unrecorded but
        // for its audit row (a late confirmation isn't audited at all).
        if landed.is_empty() {
            if let Some((_, err)) = failure {
                self.release(&origin).await;
                if !matches!(err, CommandError::NeedsConfirmation { .. }) {
                    let recorded = match err {
                        CommandError::Denied { .. } => Outcome::Denied,
                        _ => Outcome::Error,
                    };
                    self.audit_only(actor, spec, input, recorded, Some(err.to_string()))
                        .await;
                }
                return Err(err);
            }
        }

        // 3. Recorded once, with what landed (and what failed); the
        // composite's own events only when every step landed.
        if failure.is_none() {
            events.extend(plan.events.iter().cloned());
        }
        let mut result = json!({ "result": plan.result, "children": &landed });
        let stop = failure.as_ref().map(|(step, err)| {
            result["failed"] = json!({
                "name": step.call.name,
                "input": step.call.input,
                "error": err.to_string(),
            });
            stopped(&step.call.name, &landed, err)
        });
        let failed = stop.as_ref().map(ToString::to_string);
        let out = HandlerOutput {
            result: result.clone(),
            inverse: None,
            events,
            after_commit: None,
            unchanged: false,
        };
        let (actor_c, spec_c, input_c) = (actor.clone(), spec.clone(), input.clone());
        let vocabulary = self.log.vocabulary().clone();
        let failed_c = failed.clone();
        let recorded = self
            .db
            .transaction(move |tx| {
                let recorded = record_tx(
                    tx,
                    &vocabulary.current(),
                    &actor_c,
                    &spec_c,
                    &input_c,
                    &out,
                    Executed {
                        id: executed_id.clone(),
                        failed: failed_c.clone(),
                        cause: origin.cause(),
                        capabilities: capabilities.clone(),
                    },
                )?;
                match &origin {
                    RunOrigin::Call => {}
                    RunOrigin::Undo(original) => {
                        finish_undo_claim_tx(tx, *original, recorded.audit_id)?;
                    }
                    RunOrigin::Effect(key, _) => crate::effects::finished_tx(
                        tx,
                        &vocabulary.current(),
                        key,
                        &crate::effects::ran(recorded.audit_id, failed_c.clone()),
                        Some(recorded.event_id.clone()),
                    )?,
                    RunOrigin::Approval(id) => {
                        let id = *id;
                        proposal_store::finish_claim_tx(tx, id, recorded.audit_id)?;
                        log_approved_tx(
                            tx,
                            &vocabulary.current(),
                            &actor_c,
                            &spec_c,
                            id,
                            &recorded,
                        )?;
                    }
                }
                Ok(recorded)
            })
            .await;
        self.pump.wake();
        let recorded = match recorded {
            Ok(recorded) => Some(recorded),
            Err(e) => {
                tracing::error!(
                    command = %spec.id,
                    error = %e,
                    "a composite's steps ran but recording them failed; they stand unrecorded"
                );
                None
            }
        };
        match stop {
            Some(err) => Err(err),
            None => Ok(CommandOutcome {
                result,
                audit_id: recorded.as_ref().map(|r| r.audit_id),
                event_id: recorded.map(|r| r.event_id),
                inverse: None,
            }),
        }
    }

    /// One step inside oxplow: its handler in its own transaction, its
    /// events caused by the run's `command.executed` (recorded after the
    /// last step). Nothing is audited here; the run is.
    async fn run_step_tx(
        &self,
        handler: Arc<TxHandler>,
        actor: &Actor,
        input: &Value,
        cause: &oxplow_domain::EventId,
        confirmed: bool,
        gates: Gates,
    ) -> Result<(HandlerOutput, BTreeMap<String, u32>), CommandError> {
        let (actor, input, cause) = (actor.clone(), input.clone(), cause.clone());
        let vocabulary = self.log.vocabulary().clone();
        let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
        let failed_c = failed.clone();
        self.db
            .transaction(move |tx| {
                let trace = crate::host_capabilities::CapabilityTrace::default();
                let ctx = TxCtx {
                    conn: tx,
                    actor: &actor,
                    events: oxplow_db::EventCtx {
                        vocabulary: &vocabulary.current(),
                        source: actor.source(),
                        cause: Some(cause.clone()),
                    },
                    confirmed,
                    may_write: gates.may_write,
                    depth: 1,
                    trace: &trace,
                };
                match handler(&ctx, input.clone()) {
                    Ok(out) => Ok((out, trace.summary())),
                    Err(CommandError::Busy { message }) => {
                        Err(oxplow_domain::DomainError::Busy(message))
                    }
                    Err(err) => {
                        *failed_c.lock() = Some(err);
                        Err(oxplow_domain::DomainError::Invariant(
                            "a step failed; rolled back".into(),
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
}
