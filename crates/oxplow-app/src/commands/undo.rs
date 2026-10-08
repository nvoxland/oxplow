//! Undo (`.context/commands.md` "The pipeline"): the inverse an audited run
//! recorded, run through the same pipeline; the row is claimed with it so
//! two undos — or two approvals — can't both apply.

use super::*;

impl CommandBus {
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

    /// Before an `External` run with an origin: mark the audit row as
    /// being undone (`undone_by = UNDO_PENDING`), or the proposal as being
    /// approved (approved, no audit row yet). Fails when it's already
    /// undone or decided, or being so.
    pub(super) async fn claim(&self, origin: &RunOrigin) -> Result<(), CommandError> {
        match origin {
            RunOrigin::Call => Ok(()),
            RunOrigin::Undo(audit_id) => {
                let audit_id = *audit_id;
                self.db
                    .transaction(move |tx| mark_undone_tx(tx, audit_id, UNDO_PENDING))
                    .await
                    .map_err(|e| lost_race(origin, e).unwrap_or_else(CommandError::from))
            }
            RunOrigin::Approval(id) => {
                let id = *id;
                self.db
                    .transaction(move |tx| proposal_store::claim_tx(tx, id))
                    .await
                    .map_err(|e| lost_race(origin, e).unwrap_or_else(CommandError::from))
            }
            // A step will leave the transaction: claim the reaction first,
            // so a redelivery finds it — keeping what it composed when it
            // may be sent again by itself.
            RunOrigin::Effect(key, resend) => {
                let (key, resend, now) = (
                    key.clone(),
                    resend.clone(),
                    oxplow_domain::Timestamp::now().to_string(),
                );
                self.db
                    .transaction(move |tx| {
                        oxplow_db::effect_run_store::claim_tx(tx, &key, &now, resend.as_deref())
                    })
                    .await
                    .map_err(|e| lost_race(origin, e).unwrap_or_else(CommandError::from))
            }
        }
    }

    /// The `External` run failed: the row is undoable again, the proposal
    /// pending again.
    pub(super) async fn release(&self, origin: &RunOrigin) {
        let released = match *origin {
            // A claimed reaction stays `started`: the effect's runner
            // records it failed (`effect_triggers::run_reaction`), never
            // runs it again — and one cut off before that is recorded at
            // the next start (a person's retry or backfill) or by the
            // pump's redelivery (a live one).
            RunOrigin::Call | RunOrigin::Effect(..) => return,
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
pub(super) fn lost_race(
    origin: &RunOrigin,
    e: oxplow_domain::DomainError,
) -> Result<CommandError, oxplow_domain::DomainError> {
    use oxplow_domain::DomainError as D;
    if !matches!(e, D::Invariant(_) | D::Invalid(_)) {
        return Err(e);
    }
    let message = match origin {
        RunOrigin::Undo(audit_id) => format!("audit row {audit_id} was already undone"),
        RunOrigin::Approval(id) => format!("proposal:{id} was already decided"),
        RunOrigin::Effect(key, _) => format!(
            "effect `{}` {} {}",
            key.effect,
            crate::effects::ALREADY_REACTED,
            key.event_id
        ),
        RunOrigin::Call => return Err(e),
    };
    Ok(CommandError::Invalid {
        field: None,
        message,
    })
}

/// `undone_by` while an `External` undo is running (audit ids start at 1).
pub(super) const UNDO_PENDING: i64 = 0;

/// Replace a pending undo claim with the undo run's audit row.
pub(super) fn finish_undo_claim_tx(
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

/// The inverse of an audited run, if it can still be applied.
pub(super) fn undoable(row: &CommandAudit) -> Result<CommandCall, CommandError> {
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
