//! Proposals (`.context/commands.md` "Proposals"): an agent's or an effect's
//! run that needs a person's confirmation is kept for one — dry-run, kept,
//! then approved (run as the person) or declined.

use super::*;

impl CommandBus {
    /// A run that needs a confirmation it doesn't have. A person (or the
    /// system) is asked: `NeedsConfirmation`. An agent's or an effect's run
    /// (`Actor::proposes`) is kept for a person instead: dry-run (a `Tx` handler, confirmed, in a rolled-back
    /// transaction — what it would have done; an `External` one never
    /// runs), then the proposal and `command.proposed` in one transaction,
    /// and `Proposed`. A dry run that fails is the run's failure, audited
    /// like one. No audit row for a proposal: nothing ran.
    pub(super) async fn unconfirmed(
        &self,
        actor: &Actor,
        origin: RunOrigin,
        run: Prepared<'_>,
        input: Value,
        preview: Preview,
        gates: Gates,
    ) -> CommandError {
        let Prepared { command, resolved } = run;
        if !actor.proposes() {
            return CommandError::NeedsConfirmation {
                preview: Box::new(preview),
            };
        }
        // A proposal is a plain call (an effect's included); an undo kept
        // as one would lose the row it undoes (an approval is a person's,
        // so never here).
        if !matches!(origin, RunOrigin::Call | RunOrigin::Effect(..)) {
            return CommandError::Denied {
                reason: format!(
                    "`{}` needs a person's confirmation; ask them to undo it",
                    command.spec.id
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
            command: spec.id.clone(),
            input,
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            stream_id: actor.anchors().stream_id,
            preview: serde_json::to_value(&preview).expect("a preview serializes"),
            dry_run,
        };
        let (actor_c, vocabulary, destructive) = (
            actor.clone(),
            self.log.vocabulary().clone(),
            preview.destructive,
        );
        let effect = match &origin {
            RunOrigin::Effect(key, _) => Some(key.clone()),
            _ => None,
        };
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
                    &CommandProposedV2 {
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
                append_tx(tx, &vocabulary.current(), &proposed)?;
                // An effect's reaction ends here: a person decides the rest.
                if let Some(key) = &effect {
                    crate::effects::finished_tx(
                        tx,
                        &vocabulary.current(),
                        key,
                        &oxplow_db::effect_run_store::Finished {
                            state: oxplow_db::effect_run_store::RunState::Proposed,
                            reason: None,
                            audit_id: None,
                            proposal_id: Some(inserted.id),
                        },
                        Some(proposed.id.clone()),
                    )?;
                }
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
    pub(super) async fn dry_run(
        &self,
        handler: Arc<TxHandler>,
        actor: &Actor,
        input: &Value,
        gates: Gates,
    ) -> Result<Value, CommandError> {
        let (actor, input) = (actor.clone(), input.clone());
        let vocabulary = self.log.vocabulary().clone();
        self.db
            .rehearse_or(move |tx| {
                let ctx = TxCtx {
                    conn: tx,
                    actor: &actor,
                    events: oxplow_db::EventCtx {
                        vocabulary: &vocabulary.current(),
                        source: actor.source(),
                        cause: None,
                    },
                    confirmed: true,
                    may_write: gates.may_write,
                    depth: 0,
                    trace: &crate::scope_calls::ScopeTrace::default(),
                };
                match handler(&ctx, input.clone()) {
                    Ok(out) => Ok(out.result),
                    Err(CommandError::Busy { message }) => {
                        Err(TxError::Storage(oxplow_domain::DomainError::Busy(message)))
                    }
                    Err(err) => Err(TxError::Aborted(err)),
                }
            })
            .await
            .map_err(command_error)
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
        let (actor_c, vocabulary) = (actor.clone(), self.log.vocabulary().clone());
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
                append_tx(tx, &vocabulary.current(), &event)?;
                Ok(())
            })
            .await?;
        self.pump.wake();
        Ok(())
    }

    /// Proposal `id`, when `actor` is a person and it still waits.
    pub(super) async fn pending_proposal(
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
}

/// `command.approved` for proposal `id`, caused by the approving run.
pub(super) fn log_approved_tx(
    tx: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    actor: &Actor,
    spec: &CommandSpec,
    id: i64,
    recorded: &Recorded,
) -> Result<(), oxplow_domain::DomainError> {
    let approved = Envelope::typed::<CommandApproved>(
        actor.source(),
        &CommandApprovedV1 {
            proposal: proposal_ref(id),
            command: spec.id.clone(),
            audit_id: recorded.audit_id,
        },
    )
    .with_subject([proposal_ref(id), command_ref(&spec.id)])
    .with_cause(recorded.event_id.clone());
    append_tx(tx, vocabulary, &approved)?;
    Ok(())
}
