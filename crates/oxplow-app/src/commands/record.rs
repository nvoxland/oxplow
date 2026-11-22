//! Recording a run (`.context/commands.md` "The pipeline" step 5): its audit
//! row, `command.executed` and its events in one transaction, or — for a
//! run that wrote nothing — its run record alone.

use super::*;
use oxplow_domain::run::RunContext;

pub(super) fn record_effect_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    key: &oxplow_db::effect_run_store::EffectRunKey,
    effect: &RunContext,
    command: &Recorded,
    failed: Option<String>,
    may_retry: bool,
) -> Result<(), oxplow_domain::DomainError> {
    let account_health = failed.is_none() || !may_retry;
    crate::effects::finished_tx(
        tx,
        vocabulary,
        key,
        &crate::effects::ran(command.command_run_id, failed),
        Some(effect),
        Some(command.event_id.clone()),
        account_health,
    )
}

tokio::task_local! {
    /// The command detail ID of the last run that failed in this task (a caller's
    /// [`tracking_failed_run`] scope).
    static FAILED_RUN: std::cell::Cell<Option<i64>>;
}

fn note_failed_run(id: i64) {
    let _ = FAILED_RUN.try_with(|c| c.set(Some(id)));
}

/// Run `fut` and report the command detail ID if a run in it failed —
/// the last one (a composite's own, after its children's). The window's
/// front door (`oxplow_rpc::dispatch`) hands it to the window with the
/// error, so the window's record of the failure links the run.
pub async fn tracking_failed_run<F: std::future::Future>(fut: F) -> (F::Output, Option<i64>) {
    FAILED_RUN
        .scope(std::cell::Cell::new(None), async {
            let out = fut.await;
            (out, FAILED_RUN.with(|c| c.get()))
        })
        .await
}

impl CommandBus {
    /// Audit a run that wrote nothing (invalid input, a denied invoker, a
    /// failed handler). Best effort: an audit failure must not mask the
    /// error being reported.
    pub(super) async fn record_unapplied(
        &self,
        run: &RunContext,
        spec: &CommandSpec,
        input: &Value,
        outcome: Outcome,
        error: Option<String>,
    ) {
        let private_input = run.used_private_config()
            || spec.atomicity == oxplow_domain::Atomicity::Dispatch
            || spec
                .unrecorded
                .iter()
                .any(|field| field == "*" || input.get(field).is_some());
        // Schema/handler diagnostics can quote rejected values. Preserve the
        // actual error for the caller, but never copy it into public history.
        let error = if private_input {
            error.map(|_| "command with private input was rejected".into())
        } else {
            error
        };
        let row = NewCommandRun {
            command: spec.id.clone(),
            input: Some(
                if private_input || spec.atomicity == oxplow_domain::Atomicity::Dispatch {
                    super::steps::private_composite_spec(spec).recorded_input(input)
                } else {
                    spec.recorded_input(input)
                },
            ),
            outcome,
            error,
            result: None,
            inverse: None,
            scopes: Default::default(),
        };
        let run_c = run.clone();
        match self
            .db
            .transaction(move |tx| {
                oxplow_db::command_run_store::insert_rolled_back_children_tx(tx, &run_c)?;
                insert_tx(tx, &run_c, &row)
            })
            .await
        {
            Ok(id) => note_failed_run(id),
            Err(e) => tracing::warn!(error = %e, command = %spec.id, "command audit write failed"),
        }
    }

    /// Record an `External` run whose effects already happened: the audit
    /// row and `command.executed` (and, for an undo, the claimed row's
    /// real `undone_by`). If recording fails, the run still happened — it
    /// is reported as done and unrecorded (logged at error level), never
    /// as an error, which would tell the caller the change didn't happen.
    pub(super) async fn record_external(
        &self,
        run: &RunContext,
        spec: &CommandSpec,
        input: &Value,
        mut out: HandlerOutput,
        origin: RunOrigin,
        scopes: std::collections::BTreeMap<String, u32>,
    ) -> CommandOutcome {
        let (actor_c, spec_c, input_c) = (run.actor().clone(), spec.clone(), input.clone());
        let run_c = run.clone();
        let vocabulary = self.log.vocabulary().clone();
        let shadow = HandlerOutput {
            caller_result: None,
            result: out.result.clone(),
            inverse: out.inverse.clone(),
            events: out.events.clone(),
            after_commit: None,
            unchanged: false,
        };
        let recorded = self
            .db
            .transaction(move |tx| {
                let recorded = record_tx(
                    tx,
                    &vocabulary.current(),
                    &spec_c,
                    &input_c,
                    &shadow,
                    Executed {
                        scopes: scopes.clone(),
                        ..Executed::ok(oxplow_domain::EventId::generate(), &origin, &run_c)
                    },
                )?;
                match &origin {
                    RunOrigin::Call => {}
                    RunOrigin::Undo(original) => {
                        finish_undo_claim_tx(tx, *original, recorded.command_run_id)?;
                    }
                    RunOrigin::Effect(key, _, effect) => record_effect_tx(
                        tx,
                        &vocabulary.current(),
                        key,
                        effect,
                        &recorded,
                        None,
                        false,
                    )?,
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
        match recorded {
            Ok(recorded) => finish(out, recorded),
            Err(e) => {
                tracing::error!(
                    command = %spec.id,
                    error = %e,
                    "command ran but recording it failed; the change stands unrecorded"
                );
                if let Some(after) = out.after_commit.take() {
                    after();
                }
                CommandOutcome {
                    result: out.caller_result.unwrap_or(out.result),
                    command_run_id: None,
                    event_id: None,
                    inverse: None,
                }
            }
        }
    }
}

/// A transaction a handler ended with its own error: that error; else the
/// database's, as a command's.
pub(super) fn command_error(e: TxError<CommandError>) -> CommandError {
    match e {
        TxError::Aborted(e) => e,
        TxError::Storage(e) => CommandError::from(e),
    }
}

/// How a run's transaction ended short of recording it, besides the
/// database's own errors (`TxError::Storage`, busy ones retried).
pub(super) enum Abort {
    /// The handler failed: rolled back, then audited as the failure.
    Failed(CommandError),
    /// An undo or approval lost its race: nothing ran; no command detail row.
    Lost(CommandError),
    /// The call changed nothing (`HandlerOutput::unchanged`): rolled back,
    /// its answer returned, nothing recorded.
    Unchanged(Box<HandlerOutput>),
}

/// What recording a successful run produced.
pub(super) struct Recorded {
    pub(super) command_run_id: i64,
    pub(super) event_id: oxplow_domain::EventId,
    /// Undo is withheld for runs that consumed secret configuration values.
    inverse_recorded: bool,
}

/// A call that changed nothing (`HandlerOutput::unchanged`): its answer,
/// with no record.
pub(super) fn unrecorded(mut out: HandlerOutput) -> CommandOutcome {
    if let Some(after) = out.after_commit.take() {
        after();
    }
    CommandOutcome {
        result: out.caller_result.unwrap_or(out.result),
        command_run_id: None,
        event_id: None,
        inverse: None,
    }
}

pub(super) fn finish(mut out: HandlerOutput, recorded: Recorded) -> CommandOutcome {
    if let Some(after) = out.after_commit.take() {
        after();
    }
    CommandOutcome {
        result: out.caller_result.unwrap_or(out.result),
        command_run_id: Some(recorded.command_run_id),
        event_id: Some(recorded.event_id),
        inverse: out.inverse.filter(|_| recorded.inverse_recorded),
    }
}

/// The `command.executed` a run is recorded under: its id (fixed before a
/// `Tx` handler runs, so its events can name it as their cause), for a
/// composite's steps that failed partway, why, and the scopes
/// the run called.
pub(super) struct Executed {
    pub id: oxplow_domain::EventId,
    pub failed: Option<String>,
    /// What caused the run: the event an effect reacted to.
    pub cause: Option<oxplow_domain::EventId>,
    /// The run it was.
    pub run: RunContext,
    /// Its trace's summary ("Scopes").
    pub scopes: std::collections::BTreeMap<String, u32>,
}

impl Executed {
    pub(super) fn ok(id: oxplow_domain::EventId, origin: &RunOrigin, run: &RunContext) -> Self {
        Self {
            id,
            failed: None,
            cause: origin.cause(run),
            run: run.clone(),
            scopes: Default::default(),
        }
    }
}

/// Inside the run's transaction: the run and its details, `command.executed`
/// pointing at it, the handler's domain events, and the row's event id.
pub(super) fn record_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    spec: &CommandSpec,
    input: &Value,
    out: &HandlerOutput,
    executed: Executed,
) -> Result<Recorded, oxplow_domain::DomainError> {
    let Executed {
        id: executed_id,
        failed,
        cause,
        run,
        scopes,
    } = executed;
    let actor = run.actor();
    let sensitive =
        scopes.contains_key("config.secret") || run.trace().summary().contains_key("config.secret");
    let failed = if sensitive
        || spec
            .unrecorded
            .iter()
            .any(|field| field == "*" || input.get(field).is_some())
    {
        failed.map(|_| "command with private input failed".into())
    } else {
        failed
    };
    // A run that failed partway (a composite's steps, some landed) is
    // recorded with what landed, as an error.
    let outcome = if failed.is_some() {
        Outcome::Error
    } else {
        Outcome::Ok
    };
    let inverse = if spec.undoable && !sensitive {
        out.inverse.clone()
    } else {
        None
    };
    let command_run_id = insert_tx(
        tx,
        &run,
        &NewCommandRun {
            command: spec.id.clone(),
            input: Some(if sensitive {
                super::steps::private_composite_spec(spec).recorded_input(input)
            } else {
                spec.recorded_input(input)
            }),
            outcome,
            error: failed.clone(),
            // A secret configuration read returns sensitive data. A composite may
            // transform it into an arbitrary shape, so redact the entire
            // result of any run that consumed secret configuration values.
            result: Some(if sensitive {
                serde_json::json!({ "redacted": true })
            } else {
                out.result.clone()
            }),
            inverse: inverse.clone(),
            scopes,
        },
    )?;
    let mut executed = Envelope::typed::<CommandExecuted>(
        actor.source(),
        &CommandExecutedV6 {
            command: spec.id.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            outcome,
            command_run_id,
            undoable: inverse.is_some(),
        },
    )
    .with_anchors(actor.anchors())
    .with_subject([command_uri(&spec.id)]);
    executed.id = executed_id;
    executed.cause = cause;
    // An agent's command happened in the turn it has open (a person's, or a
    // page's, is in no turn).
    if actor.kind() == oxplow_domain::events::schema::ActorKind::Agent {
        if let Some(thread) = actor.thread_id() {
            let turn =
                oxplow_db::agent_stores::activity_anchors_tx(tx, thread, actor.session_id())?
                    .turn_id;
            executed.anchors.turn_id = turn;
        }
    }
    append_tx(tx, vocabulary, &executed)?;
    // A handler's own events carry the actor's thread and stream unless
    // it anchored them itself.
    let fallback = actor.anchors();
    for event in out.events.iter().filter(|_| !sensitive) {
        let mut event = event.clone().with_cause(executed.id.clone());
        event.anchors.thread_id = event.anchors.thread_id.or(fallback.thread_id);
        event.anchors.stream_id = event.anchors.stream_id.or(fallback.stream_id);
        // One already logged under its dedupe key is that same fact (a
        // re-sent step's answer): logged once.
        oxplow_db::event_log_store::append_unique_tx(tx, vocabulary, &event)?;
    }
    set_event_id_tx(tx, command_run_id, &executed.id)?;
    Ok(Recorded {
        command_run_id,
        event_id: executed.id,
        inverse_recorded: inverse.is_some(),
    })
}
