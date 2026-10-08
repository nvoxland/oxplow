//! Recording a run (`.context/commands.md` "The pipeline" step 5): its audit
//! row, `command.executed` and its events in one transaction, or — for a
//! run that wrote nothing — its audit row alone.

use super::*;

impl CommandBus {
    /// Audit a run that wrote nothing (invalid input, a denied invoker, a
    /// failed handler). Best effort: an audit failure must not mask the
    /// error being reported.
    pub(super) async fn audit_only(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        outcome: Outcome,
        error: Option<String>,
    ) {
        let row = NewCommandAudit {
            command: spec.id.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: spec.recorded_input(input),
            outcome,
            error,
            result: None,
            inverse: None,
            capabilities: Default::default(),
        };
        if let Err(e) = self
            .db
            .transaction(move |tx| insert_tx(tx, &row).map(|_| ()))
            .await
        {
            tracing::warn!(error = %e, command = %spec.id, "command audit write failed");
        }
    }

    /// Record an `External` run whose effects already happened: the audit
    /// row and `command.executed` (and, for an undo, the claimed row's
    /// real `undone_by`). If recording fails, the run still happened — it
    /// is reported as done and unrecorded (logged at error level), never
    /// as an error, which would tell the caller the change didn't happen.
    pub(super) async fn record_external(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        mut out: HandlerOutput,
        origin: RunOrigin,
        capabilities: std::collections::BTreeMap<String, u32>,
    ) -> CommandOutcome {
        let (actor_c, spec_c, input_c) = (actor.clone(), spec.clone(), input.clone());
        let vocabulary = self.log.vocabulary().clone();
        let shadow = HandlerOutput {
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
                    &actor_c,
                    &spec_c,
                    &input_c,
                    &shadow,
                    Executed {
                        capabilities: capabilities.clone(),
                        ..Executed::ok(oxplow_domain::EventId::generate(), &origin)
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
                        &crate::effects::ran(recorded.audit_id, None),
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
                    result: out.result,
                    audit_id: None,
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
    /// An undo or approval lost its race: nothing ran; no audit row.
    Lost(CommandError),
    /// The call changed nothing (`HandlerOutput::unchanged`): rolled back,
    /// its answer returned, nothing recorded.
    Unchanged(Box<HandlerOutput>),
}

/// What recording a successful run produced.
pub(super) struct Recorded {
    pub(super) audit_id: i64,
    pub(super) event_id: oxplow_domain::EventId,
}

/// A call that changed nothing (`HandlerOutput::unchanged`): its answer,
/// with no record.
pub(super) fn unrecorded(mut out: HandlerOutput) -> CommandOutcome {
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

pub(super) fn finish(mut out: HandlerOutput, recorded: Recorded) -> CommandOutcome {
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
/// `Tx` handler runs, so its events can name it as their cause), for a
/// composite's steps that failed partway, why, and the host capabilities
/// the run called.
pub(super) struct Executed {
    pub id: oxplow_domain::EventId,
    pub failed: Option<String>,
    /// What caused the run: the event an effect reacted to.
    pub cause: Option<oxplow_domain::EventId>,
    /// Its trace's summary ("Host capabilities").
    pub capabilities: std::collections::BTreeMap<String, u32>,
}

impl Executed {
    pub(super) fn ok(id: oxplow_domain::EventId, origin: &RunOrigin) -> Self {
        Self {
            id,
            failed: None,
            cause: origin.cause(),
            capabilities: Default::default(),
        }
    }
}

/// Inside the run's transaction: the audit row, `command.executed`
/// pointing at it, the handler's domain events, and the row's event id.
pub(super) fn record_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    actor: &Actor,
    spec: &CommandSpec,
    input: &Value,
    out: &HandlerOutput,
    executed: Executed,
) -> Result<Recorded, oxplow_domain::DomainError> {
    let Executed {
        id: executed_id,
        failed,
        cause,
        capabilities,
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
            command: spec.id.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: spec.recorded_input(input),
            outcome,
            error: failed,
            result: Some(out.result.clone()),
            inverse: inverse.clone(),
            capabilities,
        },
    )?;
    let mut executed = Envelope::typed::<CommandExecuted>(
        actor.source(),
        &CommandExecutedV2 {
            command: spec.id.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            outcome,
            audit_id,
            undoable: inverse.is_some(),
        },
    )
    .with_anchors(actor.anchors())
    .with_subject([command_ref(&spec.id)]);
    executed.id = executed_id;
    executed.cause = cause;
    append_tx(tx, vocabulary, &executed)?;
    // A handler's own events carry the actor's thread and stream unless
    // it anchored them itself.
    let fallback = actor.anchors();
    for event in &out.events {
        let mut event = event.clone().with_cause(executed.id.clone());
        event.anchors.thread_id = event.anchors.thread_id.or(fallback.thread_id);
        event.anchors.stream_id = event.anchors.stream_id.or(fallback.stream_id);
        // One already logged under its dedupe key is that same fact (a
        // re-sent step's answer, tsk912): logged once.
        oxplow_db::event_log_store::append_unique_tx(tx, vocabulary, &event)?;
    }
    set_event_id_tx(tx, audit_id, &executed.id)?;
    Ok(Recorded {
        audit_id,
        event_id: executed.id,
    })
}
