//! A composite's **steps** (P7 review): when one of the calls a
//! composite makes leaves the bus's transaction — an `External` command,
//! `task.*` on another provider's item — the calls can't be one
//! all-or-nothing run. They run as steps instead:
//!
//! 1. **Checked first.** Every call is routed and checked before any runs
//!    — its input, a job's tools, the agent policy — and if any asks for a
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

use oxplow_domain::run::RunContext;
use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::TxError;
use oxplow_domain::events::schema::CommandOutcome as Outcome;
use oxplow_domain::{Actor, CommandCall, CommandError, CommandOutcome, CommandSpec, Preview};
use oxplow_runtime::policy::PolicyDecision;
use serde_json::{json, Value};

use super::compose::{Compose, StepAnswer, SEQUENCE};
use super::{
    finish_undo_claim_tx, log_approved_tx, proposal_store, record_tx, Command, CommandBus,
    Executed, Gates, Handler, HandlerOutput, Invocation, NestedChild, Resolved, RunOrigin, TxCtx,
    TxHandler, MAX_NESTING,
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
    /// The run's undo, checked while routing: kept only for steps (a
    /// composition inside the transaction undoes itself).
    inverse: Option<CommandCall>,
    /// The scopes composing it called: the run's.
    scopes: BTreeMap<String, u32>,
    /// The input it was composed from, for its `finish`.
    input: Value,
    /// Its `finish`, called once every call landed: it sets the run's
    /// result and (for steps) its undo.
    finish: Option<Arc<super::compose::Finisher>>,
    transparent_result: bool,
    /// What `finish` reads through.
    reads: super::compose::Reads,
}

/// One call of a [`Composed`].
#[derive(Clone)]
pub(super) struct ComposedCall {
    command: Arc<Command>,
    call: CommandCall,
    /// Captured while routing, from this command's owning contribution.
    /// The handler and its authority never come from a later registry lookup.
    run: RunContext,
    /// A composite call's own composition.
    nested: Option<Arc<Composed>>,
}

impl Composed {
    fn has_private_input(&self) -> bool {
        self.scopes.contains_key("config.secret")
            || self.calls.iter().any(ComposedCall::has_private_input)
    }

    /// Its calls as a confirmation shows them.
    fn preview_input(&self) -> Value {
        if self.has_private_input() {
            json!({ "redacted": true })
        } else {
            json!({ "calls": self.calls.iter().map(|c| &c.call).collect::<Vec<_>>() })
        }
    }
}

impl ComposedCall {
    fn command_detail(
        &self,
        out: &HandlerOutput,
        scopes: BTreeMap<String, u32>,
    ) -> super::NewCommandRun {
        let private = self.has_private_input();
        super::NewCommandRun {
            command: self.call.name.clone(),
            input: (!out.unchanged).then(|| self.recorded_input()),
            outcome: Outcome::Ok,
            error: None,
            result: (!out.unchanged).then(|| {
                if private {
                    json!({ "redacted": true })
                } else {
                    out.result.clone()
                }
            }),
            inverse: None,
            scopes,
        }
    }

    fn has_private_input(&self) -> bool {
        self.run.trace().summary().contains_key("config.secret")
            || self
                .command
                .spec
                .unrecorded
                .iter()
                .any(|field| field == "*" || self.call.input.get(field).is_some())
            || self
                .nested
                .as_ref()
                .is_some_and(|nested| nested.has_private_input())
    }

    fn recorded_input(&self) -> Value {
        if self.run.trace().summary().contains_key("config.secret")
            || self
                .nested
                .as_ref()
                .is_some_and(|nested| nested.has_private_input())
        {
            // Arbitrary composers can derive a private child from any part of
            // their input. There is no safe field-level reverse mapping.
            private_composite_spec(&self.command.spec).recorded_input(&self.call.input)
        } else {
            self.command.spec.recorded_input(&self.call.input)
        }
    }
}

/// Preserve command behavior while withholding the entire composite input
/// from recording. This copy is never published in the registry.
pub(super) fn private_composite_spec(spec: &CommandSpec) -> CommandSpec {
    let mut private = spec.clone();
    if !private.unrecorded.iter().any(|field| field == "*") {
        private.unrecorded.push("*".into());
    }
    private
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

/// The commands only the composing extension's own script can run
/// (`commands/extension_services.rs`): a run of one by anyone else is
/// refused, as an undo would be.
const OWN_STORE_WRITES: [&str; 3] = [
    "oxplow.extension_store.write",
    "oxplow.extension_files.write",
    "oxplow.extension_files.delete",
];

/// A composition routed: what it runs, and the first call that leaves
/// the transaction, if one does (it then runs as steps).
struct Routed {
    composed: Composed,
    outside: Option<usize>,
}

/// Routes a composite's calls — composing it once, through `composing` — checking
/// each call (step 0, its input, that it writes) and collecting its check
/// before the transaction.
pub(super) struct Router<'a> {
    pub registry: &'a BTreeMap<String, Arc<Command>>,
    pub composing: &'a super::compose::Composing<'a>,
    /// The same, owned: a routed composition keeps it for its `finish`.
    pub reads: &'a super::compose::Reads,
    /// What's active: each call needs what a direct one would (step 0).
    pub active: Option<&'a crate::capabilities::Active>,
    pub prechecks: Vec<Pending>,
    /// The run being routed: each composer runs as a pure child of it.
    pub run: &'a RunContext,
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
        self.route_for(self.run, compose, input, name, depth, at)
    }

    fn route_for(
        &mut self,
        run: &RunContext,
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
        run.check_authority()
            .map_err(|reason| CommandError::Denied { reason })?;
        let composing = run.child(None, oxplow_domain::run::Mode::Pure);
        let composition = (compose.compose)(self.composing, &composing, input);
        // Read taint survives a composer failure and reaches routing, audit,
        // previews and every call derived from the returned values.
        run.trace().add(&composing.trace().summary());
        let composition = composition?;
        let routed = self.route_calls_for(run, composition.calls, depth, at);
        let (calls, outside) = routed?;
        let inverse = match composition.inverse {
            None => None,
            Some(_) if outside.is_none() => {
                return Err(CommandError::Invalid {
                    field: Some(format!("{at}/inverse")),
                    message: format!(
                        "`{name}` returned an `inverse`, but its calls stay in oxplow's records, \
                         which undo themselves; drop `inverse`"
                    ),
                })
            }
            Some(inverse) => Some(route_inverse(
                self.registry,
                self.active,
                run,
                inverse,
                name,
                at,
            )?),
        };
        Ok(Routed {
            composed: Composed {
                calls,
                result: composition.result,
                events: composition.events,
                inverse,
                scopes: run.trace().summary(),
                input: input.clone(),
                finish: compose.finish.clone(),
                transparent_result: compose.transparent_result,
                reads: self.reads.clone(),
            },
            outside,
        })
    }

    /// Route `calls`, made at nesting `depth`: each checked, a composite
    /// one composed in turn. With the first that leaves the transaction.
    fn route_calls_for(
        &mut self,
        parent: &RunContext,
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
            // A script composes its own extension's commands freely; any
            // other one a `commands:<pattern>` permission must name. A
            // settings write is waived only once its run is known to carry
            // the composer, which limits it to the composer's own keys.
            let own_settings =
                oxplow_domain::permissions::OWN_SETTINGS_WRITES.contains(&call.name.as_str());
            let decide =
                || match parent.decide(oxplow_domain::grant::Need::Command(call.name.clone())) {
                    oxplow_domain::grant::Decision::Denied(why) => Err(CommandError::Denied {
                        reason: format!("composing `{}`: {why}", call.name),
                    }),
                    _ => Ok(()),
                };
            if !own_settings {
                decide()?;
            }
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
            let mode = match command.handler {
                Handler::Tx(_) => oxplow_domain::run::Mode::Transactional,
                _ => oxplow_domain::run::Mode::External,
            };
            let run = command.run_under(parent, mode);
            if own_settings && run.composed_by().is_none() {
                decide()?;
            }
            if parent.trace().summary().contains_key("config.secret") {
                run.trace().count("config.secret");
            }
            run.check_authority()
                .map_err(|reason| CommandError::Denied { reason })?;
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
                    let inner =
                        self.route_for(&run, c, &call.input, &call.name, depth + 1, &call_at);
                    parent.trace().add(&run.trace().summary());
                    let inner = inner?;
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
                run,
                nested,
            });
        }
        Ok((routed, outside))
    }
}

/// A script's `inverse` as the one call the run's undo is: each call
/// names a registered command that writes, is one `parent` may
/// compose and its input fits; the undo runs them later, as whoever
/// undoes. One call stays as it is; several are a sequence, whose
/// steps are `Tx` or `External` commands (a composite that leaves the
/// transaction can't be a step of one).
pub(super) fn route_inverse(
    registry: &BTreeMap<String, Arc<Command>>,
    active: Option<&crate::capabilities::Active>,
    parent: &RunContext,
    calls: Vec<CommandCall>,
    name: &str,
    at: &str,
) -> Result<CommandCall, CommandError> {
    let several = calls.len() > 1;
    for (i, call) in calls.iter().enumerate() {
        let place = format!("{at}/inverse/commands/{i}");
        let invalid = |message: String| CommandError::Invalid {
            field: Some(format!("{place}/name")),
            message: format!("`{name}`'s inverse: {message}"),
        };
        let command = registry
            .get(&call.name)
            .ok_or_else(|| CommandError::Unknown {
                name: call.name.clone(),
            })?;
        super::offered(active, &command.spec, &format!("{place}/name"))?;
        if let oxplow_domain::grant::Decision::Denied(why) =
            parent.decide(oxplow_domain::grant::Need::Command(call.name.clone()))
        {
            return Err(CommandError::Denied {
                reason: format!("`{name}`'s inverse composing `{}`: {why}", call.name),
            });
        }
        if command.spec.access.reads_only() {
            return Err(invalid(format!(
                "`{}` only reads; an inverse runs commands that write",
                call.name
            )));
        }
        if OWN_STORE_WRITES.contains(&call.name.as_str()) {
            return Err(invalid(format!(
                "`{}` can only be composed by the extension's own script, which an undo \
                 is not; name the extension's own command that makes the write",
                call.name
            )));
        }
        // It records its own undo when a script composes it; named in a
        // script's inverse it would write as whatever `extension` says.
        if call.name == super::model_commands::SET_FIELDS {
            return Err(invalid(format!(
                "`{}` records its own undo when the script composes it; leave it out of \
                 the inverse",
                call.name
            )));
        }
        if several && matches!(command.handler, Handler::Compose(_)) {
            return Err(invalid(format!(
                "`{}` composes other commands, and an inverse of several calls runs as a \
                 sequence; name just that one call, or only commands that don't compose",
                call.name
            )));
        }
        command.validator.check(&call.input).map_err(|e| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field: Some(format!("{place}/input{}", field.unwrap_or_default())),
                message: format!(
                    "`{name}`'s inverse: the input doesn't fit `{}`: {message}",
                    call.name
                ),
            },
            other => other,
        })?;
    }
    Ok(match <[CommandCall; 1]>::try_from(calls) {
        Ok([only]) => only,
        Err(calls) => CommandCall {
            name: SEQUENCE.into(),
            input: json!({ "calls": calls }),
        },
    })
}

/// Every call of `calls`, and of the composites among them, as `actor`
/// may run it (`may_write`: the write gate's answer): a job's tools and
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
        if let Some(err) = super::job_refusal(spec, actor) {
            return Err(err);
        }
        if let Some(thread_id) = actor.gated_thread() {
            let gated = may_write.filter(|_| spec.access.needs_thread());
            if let PolicyDecision::Deny { reason, .. } =
                policy.check_command(thread_id.as_ref(), spec, gated)
            {
                return Err(CommandError::Denied { reason });
            }
        }
        let confirm = super::asks(&c.command, actor, &c.call.input);
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
/// nothing returns no events and no inverse.
fn honest(out: HandlerOutput, name: &str) -> Result<HandlerOutput, CommandError> {
    if out.unchanged && (!out.events.is_empty() || out.inverse.is_some()) {
        return Err(CommandError::Failed {
            message: format!("`{name}` said it changed nothing but returned events or an inverse"),
        });
    }
    Ok(out)
}

/// Run composite `parent`'s `composed` calls in `ctx`'s transaction, as
/// one: every call checked first (each one's job tools, the agent
/// policy, its confirmation — one that asks makes the run ask, unless it
/// was confirmed), then each run one level deeper.
pub(super) fn run_composed(
    policy: &AgentPolicy,
    ctx: &TxCtx<'_>,
    parent: &CommandSpec,
    composed: &Composed,
) -> Result<HandlerOutput, CommandError> {
    let (asks, destructive) = check_tree(policy, ctx.actor(), ctx.may_write, &composed.calls)?;
    if asks && !ctx.confirmed {
        return Err(CommandError::NeedsConfirmation {
            preview: Box::new(Preview {
                command: parent.id.clone(),
                summary: parent.summary.clone(),
                input: composed.preview_input(),
                destructive,
                details: Vec::new(),
            }),
        });
    }
    run_tree(ctx, &parent.id, composed)
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
fn run_tree(
    ctx: &TxCtx<'_>,
    name: &str,
    composed: &Composed,
) -> Result<HandlerOutput, CommandError> {
    ctx.trace.add(&composed.scopes);
    let mut children = Vec::with_capacity(composed.calls.len());
    let mut caller_children = Vec::with_capacity(composed.calls.len());
    let mut has_caller_result = false;
    let mut events = Vec::new();
    let mut afters: Vec<Box<dyn FnOnce() + Send + Sync>> = Vec::new();
    let mut inverses: Vec<Option<CommandCall>> = Vec::new();
    let mut answers: Vec<StepAnswer> = Vec::new();
    for c in &composed.calls {
        c.run
            .check_authority()
            .map_err(|reason| CommandError::Denied { reason })?;
        let trace = oxplow_domain::scope::ScopeTrace::default();
        let inner = TxCtx {
            conn: ctx.conn,
            run: &c.run,
            events: ctx.events.clone(),
            confirmed: ctx.confirmed,
            may_write: ctx.may_write,
            depth: ctx.depth + 1,
            trace: &trace,
        };
        let out = match (&c.nested, &c.command.handler) {
            (Some(nested), _) => run_tree(&inner, &c.call.name, nested),
            (None, Handler::Tx(handler)) => handler(&inner, c.call.input.clone()),
            (None, _) => {
                return Err(CommandError::Failed {
                    message: format!("`{}` can't run in the transaction", c.call.name),
                });
            }
        };
        c.run.trace().add(&trace.summary());
        ctx.trace.add(&c.run.trace().summary());
        if ctx.trace.summary().contains_key("config.secret")
            && !ctx.run.trace().summary().contains_key("config.secret")
        {
            ctx.run.trace().count("config.secret");
        }
        use oxplow_domain::run::Outcome as RunOutcome;
        let checked = out.and_then(|out| honest(out, &c.call.name));
        let outcome = match &checked {
            Ok(out) if out.unchanged => RunOutcome::Ok,
            Ok(_) => RunOutcome::Cancelled,
            Err(CommandError::Denied { .. }) => RunOutcome::Denied,
            Err(CommandError::Invalid { .. }) => RunOutcome::Invalid,
            Err(_) => RunOutcome::Failed,
        };
        let error = checked.as_ref().err().map(|error| {
            if ctx.run.used_private_config() || c.run.used_private_config() {
                "command using private configuration failed".to_owned()
            } else {
                error.to_string()
            }
        });
        c.run.observe_rollback_outcome(outcome, error);
        let mut out = checked?;
        if ctx.trace.summary().contains_key("config.secret")
            && !c.run.trace().summary().contains_key("config.secret")
        {
            c.run.trace().count("config.secret");
        }
        super::insert_tx(
            ctx.conn,
            &c.run,
            &c.command_detail(&out, c.run.trace().summary()),
        )?;
        if let Some(after) = out.after_commit.take() {
            afters.push(after);
        }
        events.extend(out.events);
        has_caller_result |= out.caller_result.is_some();
        let child_result = out
            .caller_result
            .clone()
            .unwrap_or_else(|| out.result.clone());
        caller_children.push(json!({"name":c.call.name,"result":child_result}));
        if composed.finish.is_some() {
            answers.push(StepAnswer {
                name: c.call.name.clone(),
                input: c.call.input.clone(),
                result: child_result,
            });
        }
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
    // Its `finish` runs here, still in the transaction, so a refusal rolls
    // every child back: nothing is left landed.
    let mut result = composed.result.clone();
    if let Some(finish) = &composed.finish {
        let reading = ctx.run.child(None, oxplow_domain::run::Mode::Pure);
        // The transaction holds the database: a read through the gateway
        // would wait on it, so `finish` has no reads here.
        let done = finish(None, &reading, &composed.input, &answers);
        ctx.trace.add(&reading.trace().summary());
        ctx.run.trace().add(&reading.trace().summary());
        let done = done?;
        if done.inverse.is_some() {
            return Err(CommandError::Invalid {
                field: Some("/inverse".into()),
                message: format!(
                    "`{name}`'s finish returned an `inverse`, but its calls stay in oxplow's \
                     records, which undo themselves; drop `inverse` from finish (it can set \
                     `result`). Nothing ran"
                ),
            });
        }
        if done.result.is_some() {
            result = done.result;
        }
    }
    Ok(HandlerOutput {
        caller_result: if composed.transparent_result {
            Some(result.clone().unwrap_or(Value::Null))
        } else if has_caller_result {
            Some(json!({"result":result,"children":caller_children}))
        } else {
            None
        },
        result: json!({ "result": result, "children": children }),
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
/// (`record_tx` skips an event already logged under its key).
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
/// ones that landed before it, which stand — of the failing step's kind:
/// `Unavailable` (worth sending again) stays so, with its wait,
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

/// A `finish` that failed after every step landed, as the run reports
/// it: the steps stand.
fn finish_stopped(name: &str, landed_children: &[NestedChild], err: &CommandError) -> CommandError {
    let landed: Vec<String> = landed_children
        .iter()
        .map(|c| format!("`{}`", c.name))
        .collect();
    // What the steps answered with rows (a store write's `RETURNING`, the
    // minted id) stays readable although the run failed.
    let answered: Vec<String> = landed_children
        .iter()
        .filter(|c| c.result["rows"].as_array().is_some_and(|r| !r.is_empty()))
        .map(|c| format!("`{}` answered rows {}", c.name, c.result["rows"]))
        .collect();
    let answered = if answered.is_empty() {
        String::new()
    } else {
        format!(" ({})", answered.join("; "))
    };
    let message = format!(
        "`{name}`'s finish failed after {} ran — they stand{answered}, and the run can't be undone: {err}",
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
    /// Call a steps run's `finish` over the answers of the steps that
    /// landed, off the async runtime (it reads through the SQL gateway as
    /// the composer did), and check its `inverse` as routing checks a
    /// script's (a known writing command, permitted, needs met, an input
    /// that fits). The scopes it read are added to the run's.
    async fn finish_steps(
        &self,
        run: &RunContext,
        spec: &CommandSpec,
        plan: &Arc<Composed>,
        finish: &Arc<super::compose::Finisher>,
        landed: &[NestedChild],
        scopes: &mut BTreeMap<String, u32>,
    ) -> Result<(Option<Value>, Option<CommandCall>), CommandError> {
        let answers: Vec<StepAnswer> = landed
            .iter()
            .zip(&plan.calls)
            .map(|(child, step)| StepAnswer {
                name: child.name.clone(),
                input: step.call.input.clone(),
                result: child.result.clone(),
            })
            .collect();
        let (finish, plan_c) = (finish.clone(), plan.clone());
        let reading = run.child(None, oxplow_domain::run::Mode::Pure);
        let reading_c = reading.clone();
        let done = crate::extension::extension_activation::ExtensionActivation::spawn_blocking(
            move || {
                finish(
                    Some(&plan_c.reads.composing()),
                    &reading_c,
                    &plan_c.input,
                    &answers,
                )
            },
        )
        .await
        .map_err(|e| CommandError::Failed {
            message: format!("finish failed: {e}"),
        });
        oxplow_domain::scope::add_counts(scopes, reading.trace().summary());
        run.trace().add(&reading.trace().summary());
        let done = done??;
        let inverse = match done.inverse {
            None => None,
            Some(calls) => {
                let registry = self.commands.read().commands.clone();
                let active = self.active();
                Some(route_inverse(
                    &registry,
                    active.as_ref(),
                    run,
                    calls,
                    &spec.id,
                    "",
                )?)
            }
        };
        Ok((done.result, inverse))
    }

    /// Route composite `spec` (`compose`) for `input`: composed once, on a
    /// blocking thread reading through the SQL gateway, each call checked
    /// and routed. Every call inside the
    /// transaction → a `Tx` handler running exactly those calls; any
    /// outside → its steps. With it, every composed call's check before
    /// the transaction.
    pub(super) async fn route_composite(
        &self,
        run: &RunContext,
        spec: &CommandSpec,
        compose: &Arc<Compose>,
        input: &Value,
    ) -> Result<(Resolved, Vec<Pending>, bool), CommandError> {
        let registry = self.commands.read().commands.clone();
        let active = self.active();
        let (c, input, run) = (compose.clone(), input.clone(), run.clone());
        let name = spec.id.clone();
        let (sql, db) = (self.sql.clone(), self.db.clone());
        let runtime = tokio::runtime::Handle::current();
        let routed = crate::extension::extension_activation::ExtensionActivation::spawn_blocking(
            move || {
                let reads = super::compose::Reads { sql, db, runtime };
                let composing = reads.composing();
                let mut router = Router {
                    registry: &registry,
                    composing: &composing,
                    reads: &reads,
                    active: active.as_ref(),
                    prechecks: Vec::new(),
                    run: &run,
                };
                router
                    .route(&c, &input, &name, 0, "")
                    .map(|routed| (routed, router.prechecks))
            },
        )
        .await
        .map_err(|e| CommandError::Failed {
            message: format!("composing failed: {e}"),
        })??;
        let (Routed { composed, outside }, prechecks) = routed;
        let composed = Arc::new(composed);
        let private_input = composed.has_private_input();
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
            private_input,
        ))
    }

    /// Run `plan`'s calls as steps, as `actor` (see the module doc).
    pub(super) async fn run_steps(
        &self,
        run: &RunContext,
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
        let actor = run.actor();
        // 1. Every step's — and every composite step's calls' — own
        // a job's tools, policy and confirmation, before any runs.
        let (asks, destructive) =
            match check_tree(&self.policy, actor, gates.may_write, &plan.calls) {
                Ok(answer) => answer,
                Err(err) => {
                    self.record_unapplied(run, spec, input, Outcome::Denied, Some(err.to_string()))
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
                    details: Vec::new(),
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
            if let Err(reason) = step.run.check_authority() {
                failure = Some((step, CommandError::Denied { reason }));
                break;
            }
            let tx = match (&step.nested, &step.command.handler) {
                (Some(nested), _) => Some(composed_handler(
                    self.policy.clone(),
                    step.command.spec.clone(),
                    nested.clone(),
                )),
                (None, Handler::Tx(handler)) => Some(handler.clone()),
                (None, _) => None,
            };
            let recorded_in_transaction = tx.is_some();
            let ran = match (tx, &step.command.handler) {
                (Some(handler), _) => {
                    self.run_step_tx(handler, step, &executed_id, confirmed, gates)
                        .await
                }
                (None, Handler::External(handler)) => {
                    let invocation = Invocation {
                        run: step.run.clone(),
                        ..origin.invocation(&step.run, index, &step.call.name, &step.call.input)
                    };
                    let key = invocation.idempotency_key.clone();
                    let called = invocation.run.clone();
                    let answer = handler(invocation, step.call.input.clone()).await;
                    step.run.trace().add(&called.trace().summary());
                    answer.map(|out| {
                        (
                            same_events_once(out, key.as_deref()),
                            called.trace().summary(),
                        )
                    })
                }
                (None, _) => Err(CommandError::Failed {
                    message: format!("`{}` has no handler to run", step.call.name),
                }),
            };
            run.trace().add(&step.run.trace().summary());
            oxplow_domain::scope::add_counts(&mut scopes, step.run.trace().summary());
            let ran = ran.and_then(|(out, used)| Ok((honest(out, &step.call.name)?, used)));
            match ran {
                Ok((mut out, used)) => {
                    if !recorded_in_transaction {
                        let row = step.command_detail(&out, used.clone());
                        let child = step.run.clone();
                        if let Err(error) = self
                            .db
                            .transaction(move |tx| super::insert_tx(tx, &child, &row).map(|_| ()))
                            .await
                        {
                            tracing::error!(%error, command = %step.call.name, "external step landed without a command run record");
                        }
                    }
                    oxplow_domain::scope::add_counts(&mut scopes, used);
                    if let Some(after) = out.after_commit.take() {
                        after();
                    }
                    events.extend(out.events);
                    landed.push(NestedChild {
                        name: step.call.name.clone(),
                        input: step.recorded_input(),
                        result: out.result,
                        inverse: out.inverse.filter(|_| step.command.spec.undoable),
                    });
                }
                Err(err) => {
                    if !matches!(&err, CommandError::NeedsConfirmation { .. }) {
                        self.record_unapplied(
                            &step.run,
                            &step.command.spec,
                            &step.call.input,
                            if matches!(&err, CommandError::Denied { .. }) {
                                Outcome::Denied
                            } else if matches!(&err, CommandError::Invalid { .. }) {
                                Outcome::Invalid
                            } else {
                                Outcome::Error
                            },
                            Some(err.to_string()),
                        )
                        .await;
                    }
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
                    self.record_unapplied(run, spec, input, recorded, Some(err.to_string()))
                        .await;
                }
                return Err(err);
            }
        }

        // 3. Its `finish`, when every step landed: the run's result and
        // undo from what the steps answered. A finish that fails (or whose
        // inverse is refused) leaves the steps landed and fails the run.
        let mut plan_result = plan.result.clone();
        let mut plan_inverse = plan.inverse.clone();
        let mut finish_failure: Option<CommandError> = None;
        if failure.is_none() {
            if let Some(finish) = &plan.finish {
                match self
                    .finish_steps(run, spec, &plan, finish, &landed, &mut scopes)
                    .await
                {
                    Ok((result, inverse)) => {
                        if result.is_some() {
                            plan_result = result;
                        }
                        if inverse.is_some() {
                            plan_inverse = inverse;
                        }
                    }
                    Err(err) => finish_failure = Some(err),
                }
            }
        }

        // 4. Recorded once, with what landed (and what failed); the
        // composite's own events only when every step landed.
        if failure.is_none() && finish_failure.is_none() {
            events.extend(plan.events.iter().cloned());
        }
        let caller_result = plan
            .transparent_result
            .then(|| plan_result.clone().unwrap_or(Value::Null));
        let mut result = json!({ "result": plan_result, "children": &landed });
        let stop = failure.as_ref().map(|(step, err)| {
            result["failed"] = json!({
                "name": step.call.name,
                "input": step.recorded_input(),
                "error": if step.has_private_input() {
                    "private command failed".to_string()
                } else {
                    err.to_string()
                },
            });
            stopped(&step.call.name, &landed, err)
        });
        let stop = stop.or_else(|| {
            finish_failure.as_ref().map(|err| {
                result["failed"] = json!({
                    "name": format!("{}.finish", spec.id),
                    "error": err.to_string(),
                });
                finish_stopped(&spec.id, &landed, err)
            })
        });
        let sensitive = scopes.contains_key("config.secret")
            || run.trace().summary().contains_key("config.secret");
        let failed = stop.as_ref().map(|err| {
            if sensitive {
                "command using private configuration failed".to_owned()
            } else {
                err.to_string()
            }
        });
        // The script's inverse is the undo of a run that all landed.
        let inverse = plan_inverse.filter(|_| stop.is_none() && !sensitive);
        let out = HandlerOutput {
            caller_result: None,
            result: result.clone(),
            inverse: inverse.clone(),
            events,
            after_commit: None,
            unchanged: false,
        };
        let (actor_c, spec_c, input_c) = (actor.clone(), spec.clone(), input.clone());
        let run_c = run.clone();
        let vocabulary = self.log.vocabulary().clone();
        let failed_c = failed.clone();
        let retry_after = match &stop {
            Some(CommandError::Unavailable { retry_after_ms, .. }) => Some(*retry_after_ms),
            _ => None,
        };
        let recorded = self
            .db
            .transaction(move |tx| {
                let recorded = record_tx(
                    tx,
                    &vocabulary.current(),
                    &spec_c,
                    &input_c,
                    &out,
                    Executed {
                        id: executed_id.clone(),
                        failed: failed_c.clone(),
                        cause: origin.cause(&run_c),
                        run: run_c.clone(),
                        scopes: scopes.clone(),
                    },
                )?;
                match &origin {
                    RunOrigin::Call => {}
                    RunOrigin::Undo(original) => {
                        finish_undo_claim_tx(tx, *original, recorded.command_run_id)?;
                    }
                    RunOrigin::Effect(key, resend, effect) => {
                        let retry = resend.as_ref().zip(retry_after).map(|(resend, asked)| {
                            oxplow_db::effect_run_store::AutomaticRetry {
                                resend: resend.to_string(),
                                asked: asked.map(std::time::Duration::from_millis),
                                from: oxplow_domain::Timestamp::now(),
                            }
                        });
                        super::record_effect_tx(
                            tx,
                            &vocabulary.current(),
                            key,
                            effect,
                            &recorded,
                            failed_c.clone(),
                            retry.as_ref(),
                        )?;
                    }
                    RunOrigin::Approval(id) => {
                        let id = *id;
                        proposal_store::finish_claim_tx(tx, id, recorded.command_run_id)?;
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
                result: caller_result.unwrap_or(result),
                command_run_id: recorded.as_ref().map(|r| r.command_run_id),
                event_id: recorded.map(|r| r.event_id),
                inverse,
            }),
        }
    }

    /// One step inside oxplow: its handler in its own transaction, its
    /// events caused by the run's `command.executed` (recorded after the
    /// last step). The step's run and detail land with its own write.
    async fn run_step_tx(
        &self,
        handler: Arc<TxHandler>,
        step: &ComposedCall,
        cause: &oxplow_domain::EventId,
        confirmed: bool,
        gates: Gates,
    ) -> Result<(HandlerOutput, BTreeMap<String, u32>), CommandError> {
        let (step, cause) = (step.clone(), cause.clone());
        let vocabulary = self.log.vocabulary().clone();
        self.db
            .transaction_or(move |tx| {
                let run = &step.run;
                let trace = oxplow_domain::scope::ScopeTrace::default();
                let ctx = TxCtx {
                    conn: tx,
                    run,
                    events: oxplow_db::EventCtx {
                        vocabulary: &vocabulary.current(),
                        source: run.actor().source(),
                        cause: Some(cause.clone()),
                    },
                    confirmed,
                    may_write: gates.may_write,
                    depth: 1,
                    trace: &trace,
                };
                let answer = handler(&ctx, step.call.input.clone())
                    .and_then(|out| honest(out, &step.call.name));
                run.trace().add(&trace.summary());
                match answer {
                    Ok(out) => {
                        let used = trace.summary();
                        super::insert_tx(tx, run, &step.command_detail(&out, used.clone()))
                            .map_err(TxError::Storage)?;
                        Ok((out, used))
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn child(name: &str, result: Value) -> NestedChild {
        NestedChild {
            name: name.into(),
            input: json!({}),
            result,
            inverse: None,
        }
    }

    /// A finish that fails after its steps landed still tells the caller what
    /// they answered: a minted id is in a store write's `rows`.
    #[test]
    fn a_failed_finish_names_the_rows_its_landed_steps_answered() {
        let landed = [
            child(
                "oxplow.extension_store.write",
                json!({ "changes": 1, "rows": [[{ "id": "tsk7" }]] }),
            ),
            child("oxplow.task.transition", json!({ "uri": "u" })),
        ];
        let said = finish_stopped(
            "x.make",
            &landed,
            &CommandError::Failed {
                message: "boom".into(),
            },
        )
        .to_string();
        assert!(said.contains("they stand"), "{said}");
        assert!(
            said.contains("oxplow.extension_store.write") && said.contains("tsk7"),
            "{said}"
        );
        // A step that answered no rows adds nothing.
        assert!(!said.contains("oxplow.task.transition` answered"), "{said}");
    }
}
