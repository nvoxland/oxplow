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
//! Composites routed entirely inside the transaction run as one
//! transaction instead ([`run_composed`]). Either way the composition is
//! made once — on the snapshot it's routed and checked on ([`Router`]) —
//! and is what runs: a [`Composed`] tree, each composite call's own
//! composition beside it, so the calls prechecked are the calls run and a
//! nested composite's confirmations are known before anything lands.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::TxError;
use oxplow_domain::events::schema::CommandOutcome as Outcome;
use oxplow_domain::{Actor, CommandCall, CommandError, CommandOutcome, CommandSpec, Preview};
use oxplow_runtime::policy::PolicyDecision;
use serde_json::{json, Value};

use super::compose::{Compose, SEQUENCE};
use super::{
    finish_undo_claim_tx, log_approved_tx, proposal_store, record_tx, Command, CommandBus,
    Executed, Gates, Handler, HandlerOutput, NestedChild, Resolved, RunOrigin, TxCtx, TxHandler,
    MAX_NESTING,
};
use crate::agent_policy::AgentPolicy;

/// A composition as routed: composed once, on the snapshot the run is
/// checked on, and run as is — its calls, each with the command that runs
/// it and, for a composite, that composite's own composition.
pub(super) struct Composed {
    calls: Vec<ComposedCall>,
    result: Option<Value>,
    /// The composite's own events, recorded with the run.
    events: Vec<oxplow_domain::Envelope>,
    /// The scopes composing it called: the run's.
    scopes: BTreeMap<String, u32>,
}

/// One call of a [`Composed`].
pub(super) struct ComposedCall {
    command: Arc<Command>,
    call: CommandCall,
    /// A composite call's own composition.
    nested: Option<Arc<Composed>>,
}

impl Composed {
    /// Calls routed by hand (`CommandBus::run_nested`): no result or events
    /// of their own, nothing composing them called.
    pub(super) fn of(calls: Vec<ComposedCall>) -> Self {
        Self {
            calls,
            result: None,
            events: Vec::new(),
            scopes: BTreeMap::new(),
        }
    }

    /// Its calls as a confirmation shows them.
    fn preview_input(&self) -> Value {
        json!({ "calls": self.calls.iter().map(|c| &c.call).collect::<Vec<_>>() })
    }
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

/// A composition routed: what it runs, and the first call that leaves
/// the transaction, if one does (it then runs as steps).
struct Routed {
    composed: Composed,
    outside: Option<usize>,
}

/// Routes a composite's calls — composing it once, on `conn` — checking
/// each call (step 0, its input, that it writes) and collecting its check
/// before the transaction (tsk1010).
pub(super) struct Router<'a> {
    pub registry: &'a BTreeMap<String, Arc<Command>>,
    pub conn: &'a rusqlite::Connection,
    /// What's active: each call needs what a direct one would (step 0).
    pub active: Option<&'a crate::capabilities::Active>,
    pub prechecks: Vec<Pending>,
}

impl Router<'_> {
    /// Compose `compose` for `input` and route each call; a call that is
    /// itself a composite must route inside the transaction (its steps
    /// would otherwise land outside the run they're a step of). Problems
    /// and checks are placed at `at` + the call's place.
    fn route(
        &mut self,
        compose: &Compose,
        input: &Value,
        name: &str,
        depth: usize,
        at: &str,
    ) -> Result<Routed, CommandError> {
        if depth >= MAX_NESTING {
            return Err(CommandError::Invalid {
                field: None,
                message: format!(
                    "`{name}` is nested more than {MAX_NESTING} composites deep — does a command \
                 compose itself?"
                ),
            });
        }
        let trace = oxplow_domain::scope::ScopeTrace::default();
        let composition = (compose.compose)(self.conn, &trace, input)?;
        let (calls, outside) = self.route_calls(composition.calls, depth, at)?;
        Ok(Routed {
            composed: Composed {
                calls,
                result: composition.result,
                events: composition.events,
                scopes: trace.summary(),
            },
            outside,
        })
    }

    /// Route `calls`, made at nesting `depth`: each checked, a composite
    /// one composed in turn. With the first that leaves the transaction.
    pub(super) fn route_calls(
        &mut self,
        calls: Vec<CommandCall>,
        depth: usize,
        at: &str,
    ) -> Result<(Vec<ComposedCall>, Option<usize>), CommandError> {
        let registry = self.registry;
        let mut routed = Vec::with_capacity(calls.len());
        let mut outside = None;
        for (i, call) in calls.into_iter().enumerate() {
            let name_field = format!("{at}/calls/{i}/name");
            let call_at = format!("{at}/calls/{i}/input");
            let at_input = |e: CommandError| match e {
                CommandError::Invalid { field, message } => CommandError::Invalid {
                    field: Some(format!("{call_at}{}", field.unwrap_or_default())),
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
            super::offered(self.active, &command.spec, &name_field)?;
            if command.spec.access.reads_only() {
                return Err(CommandError::Invalid {
                    field: Some(name_field),
                    message: format!(
                        "`{}` only reads; a composite composes commands that write",
                        call.name
                    ),
                });
            }
            command.validator.check(&call.input).map_err(at_input)?;
            if let Some(check) = &command.precheck {
                self.prechecks.push(Pending {
                    check: check.clone(),
                    input: call.input.clone(),
                    at: call_at.clone(),
                });
            }
            let nested = match &command.handler {
                Handler::Tx(_) => None,
                Handler::External(_) => {
                    outside.get_or_insert(i);
                    None
                }
                Handler::Compose(c) => {
                    let inner = self.route(c, &call.input, &call.name, depth + 1, &call_at)?;
                    if inner.outside.is_some() {
                        return Err(CommandError::Invalid {
                            field: Some(name_field),
                            message: format!(
                                "`{}` runs steps outside the transaction, so it can't be one \
                                 step of another composite",
                                call.name
                            ),
                        });
                    }
                    Some(Arc::new(inner.composed))
                }
            };
            routed.push(ComposedCall {
                command,
                call,
                nested,
            });
        }
        Ok((routed, outside))
    }
}

/// Every call of `calls`, and of the composites among them, as `actor`
/// may run it (`may_write`: the write gate's answer): its own invokers and
/// the agent policy — a composite never widens what its calls allow —
/// and whether any asks for a confirmation (and is destructive).
fn check_tree(
    policy: &AgentPolicy,
    actor: &Actor,
    may_write: Option<bool>,
    calls: &[ComposedCall],
) -> Result<(bool, bool), CommandError> {
    let (mut asks, mut destructive) = (false, false);
    for c in calls {
        let spec = &c.command.spec;
        if !spec.invokers.allows(actor.invoker()) {
            return Err(CommandError::Denied {
                reason: format!("`{}` is not open to {:?} callers", spec.id, actor.invoker()),
            });
        }
        if let Some(thread_id) = actor.gated_thread() {
            let gated = may_write.filter(|_| spec.access.needs_writer());
            if let PolicyDecision::Deny { reason, .. } =
                policy.check_command(thread_id.as_ref(), spec, gated)
            {
                return Err(CommandError::Denied { reason });
            }
        }
        let confirm = c.command.confirm(&c.call.input);
        asks |= confirm.required();
        destructive |= matches!(confirm, oxplow_domain::Confirm::Destructive);
        if let Some(nested) = &c.nested {
            let (a, d) = check_tree(policy, actor, may_write, &nested.calls)?;
            asks |= a;
            destructive |= d;
        }
    }
    Ok((asks, destructive))
}

/// A handler's answer, held to the contract: one that says it changed
/// nothing returns no events and no inverse (tsk901).
fn honest(out: HandlerOutput, name: &str) -> Result<HandlerOutput, CommandError> {
    if out.unchanged && (!out.events.is_empty() || out.inverse.is_some()) {
        return Err(CommandError::Failed {
            message: format!("`{name}` said it changed nothing but returned events or an inverse"),
        });
    }
    Ok(out)
}

/// Run composite `parent`'s `composed` calls in `ctx`'s transaction, as
/// one: every call checked first (each one's own invokers, the agent
/// policy, its confirmation — one that asks makes the run ask, unless it
/// was confirmed), then each run one level deeper.
pub(super) fn run_composed(
    policy: &AgentPolicy,
    ctx: &TxCtx<'_>,
    parent: &CommandSpec,
    composed: &Composed,
) -> Result<HandlerOutput, CommandError> {
    let (asks, destructive) = check_tree(policy, ctx.actor, ctx.may_write, &composed.calls)?;
    if asks && !ctx.confirmed {
        return Err(CommandError::NeedsConfirmation {
            preview: Box::new(Preview {
                command: parent.id.clone(),
                summary: parent.summary.clone(),
                input: composed.preview_input(),
                destructive,
            }),
        });
    }
    run_tree(ctx, composed)
}

/// `composed`'s calls, run on `ctx` one level deeper, checked already:
/// the composite's answer — each child's name and result (the caller sent
/// the inputs; the composite's inverse is the undo, so echoing either only
/// costs the reader), its events and its own after them, the children's
/// `after_commit`s in order. Its inverse is the inverses of the children
/// that changed something, reversed, as a `oxplow.command.sequence` — none
/// when one of those has none (it can't be undone in part); a child that
/// changed nothing has nothing to undo. One whose every child changed
/// nothing, with no events of its own, changed nothing.
fn run_tree(ctx: &TxCtx<'_>, composed: &Composed) -> Result<HandlerOutput, CommandError> {
    ctx.trace.add(&composed.scopes);
    let inner = TxCtx {
        conn: ctx.conn,
        actor: ctx.actor,
        events: ctx.events.clone(),
        confirmed: ctx.confirmed,
        may_write: ctx.may_write,
        depth: ctx.depth + 1,
        trace: ctx.trace,
    };
    let mut children = Vec::with_capacity(composed.calls.len());
    let mut events = Vec::new();
    let mut afters: Vec<Box<dyn FnOnce() + Send + Sync>> = Vec::new();
    let mut inverses: Vec<Option<CommandCall>> = Vec::new();
    for c in &composed.calls {
        let out = match (&c.nested, &c.command.handler) {
            (Some(nested), _) => run_tree(&inner, nested)?,
            (None, Handler::Tx(handler)) => handler(&inner, c.call.input.clone())?,
            (None, _) => {
                return Err(CommandError::Failed {
                    message: format!("`{}` can't run in the transaction", c.call.name),
                })
            }
        };
        let mut out = honest(out, &c.call.name)?;
        if let Some(after) = out.after_commit.take() {
            afters.push(after);
        }
        events.extend(out.events);
        children.push(json!({ "name": c.call.name, "result": out.result }));
        if !out.unchanged {
            inverses.push(out.inverse.filter(|_| c.command.spec.undoable));
        }
    }
    let unchanged = inverses.is_empty() && composed.events.is_empty();
    let inverse = (!inverses.is_empty())
        .then(|| inverses.into_iter().rev().collect::<Option<Vec<_>>>())
        .flatten()
        .map(|calls| CommandCall {
            name: SEQUENCE.into(),
            input: json!({ "calls": calls }),
        });
    events.extend(composed.events.iter().cloned());
    Ok(HandlerOutput {
        result: json!({ "result": composed.result, "children": children }),
        inverse,
        events,
        after_commit: (!afters.is_empty()).then(|| {
            Box::new(move || {
                for after in afters {
                    after();
                }
            }) as Box<dyn FnOnce() + Send + Sync>
        }),
        unchanged,
    })
}

/// The `Tx` handler that runs composite `parent`'s routed `composed` calls
/// — the same calls on every attempt of its transaction.
pub(super) fn composed_handler(
    policy: Arc<AgentPolicy>,
    parent: CommandSpec,
    composed: Arc<Composed>,
) -> Arc<TxHandler> {
    Arc::new(move |ctx: &TxCtx<'_>, _input: Value| run_composed(&policy, ctx, &parent, &composed))
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
    /// Route composite `spec` (`compose`) for `input`: composed once, on a
    /// read snapshot, each call checked and routed. Every call inside the
    /// transaction → a `Tx` handler running exactly those calls; any
    /// outside → its steps. With it, every composed call's check before
    /// the transaction (tsk1010).
    pub(super) async fn route_composite(
        &self,
        spec: &CommandSpec,
        compose: &Arc<Compose>,
        input: &Value,
    ) -> Result<(Resolved, Vec<Pending>), CommandError> {
        let registry = self.commands.read().commands.clone();
        let active = self.active();
        let (c, input) = (compose.clone(), input.clone());
        let routed = self
            .db
            .read_or(move |conn| {
                let mut router = Router {
                    registry: &registry,
                    conn,
                    active: active.as_ref(),
                    prechecks: Vec::new(),
                };
                router
                    .route(&c, &input, "", 0, "")
                    .map(|routed| (routed, router.prechecks))
                    .map_err(TxError::Aborted)
            })
            .await
            .map_err(super::command_error)?;
        let (Routed { composed, outside }, prechecks) = routed;
        let composed = Arc::new(composed);
        Ok((
            match outside {
                None => Resolved::Tx(composed_handler(
                    self.policy.clone(),
                    spec.clone(),
                    composed,
                )),
                Some(_) => Resolved::Steps(composed),
            },
            prechecks,
        ))
    }

    /// Run `plan`'s calls as steps, as `actor` (see the module doc).
    pub(super) async fn run_steps(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        plan: Arc<Composed>,
        admitted: Admitted,
    ) -> Result<CommandOutcome, CommandError> {
        let Admitted {
            confirmed,
            gates,
            origin,
        } = admitted;
        // 1. Every step's — and every composite step's calls' — own
        // invokers, policy and confirmation, before any runs.
        let (asks, destructive) =
            match check_tree(&self.policy, actor, gates.may_write, &plan.calls) {
                Ok(answer) => answer,
                Err(err) => {
                    self.audit_only(actor, spec, input, Outcome::Denied, Some(err.to_string()))
                        .await;
                    return Err(err);
                }
            };
        if asks && !confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: spec.id.clone(),
                    summary: spec.summary.clone(),
                    input: plan.preview_input(),
                    destructive,
                }),
            });
        }

        // 2. In order, each landing as it runs; the first failure stops.
        self.claim(&origin).await?;
        let executed_id = oxplow_domain::EventId::generate();
        let mut landed: Vec<NestedChild> = Vec::with_capacity(plan.calls.len());
        let mut events = Vec::new();
        let mut failure: Option<(&ComposedCall, CommandError)> = None;
        let mut scopes = plan.scopes.clone();
        for (index, step) in plan.calls.iter().enumerate() {
            let tx = match (&step.nested, &step.command.handler) {
                (Some(nested), _) => Some(composed_handler(
                    self.policy.clone(),
                    step.command.spec.clone(),
                    nested.clone(),
                )),
                (None, Handler::Tx(handler)) => Some(handler.clone()),
                (None, _) => None,
            };
            let ran = match (tx, &step.command.handler) {
                (Some(handler), _) => {
                    self.run_step_tx(
                        handler,
                        actor,
                        &step.call.input,
                        &executed_id,
                        confirmed,
                        gates,
                    )
                    .await
                }
                (None, Handler::External(handler)) => {
                    let invocation =
                        origin.invocation(actor, index, &step.call.name, &step.call.input);
                    let key = invocation.idempotency_key.clone();
                    let trace = invocation.trace.clone();
                    handler(invocation, step.call.input.clone())
                        .await
                        .map(|out| (same_events_once(out, key.as_deref()), trace.summary()))
                }
                (None, _) => Err(CommandError::Failed {
                    message: format!("`{}` has no handler to run", step.call.name),
                }),
            };
            let ran = ran.and_then(|(out, used)| Ok((honest(out, &step.call.name)?, used)));
            match ran {
                Ok((mut out, used)) => {
                    oxplow_domain::scope::add_counts(&mut scopes, used);
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
                        scopes: scopes.clone(),
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
        self.db
            .transaction_or(move |tx| {
                let trace = oxplow_domain::scope::ScopeTrace::default();
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
                        Err(TxError::Storage(oxplow_domain::DomainError::Busy(message)))
                    }
                    Err(err) => Err(TxError::Aborted(err)),
                }
            })
            .await
            .map_err(super::command_error)
    }
}
