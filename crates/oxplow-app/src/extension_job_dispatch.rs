//! One fenced extension-job dispatch attempt.
//!
//! Scheduling and worker wake-up live elsewhere. This coordinator claims one
//! queued row, authorizes it against current host state, and supervises one
//! registered operation without holding a database transaction across awaits.

use std::{sync::Arc, time::Duration};

use oxplow_db::{
    extension_job_store::{
        transition_tx, ExtensionJobStoreError, LateOutcomeObservation, RecoveryCursor,
        SqliteExtensionJobStore, StoredExtensionJob,
    },
    map_sql_err, Database, TxError,
};
use oxplow_domain::{
    extension_job::{AttemptFence, Completion, JobEvent, JobId, JobState},
    extension_job_context::JobExecutionIdentity,
    time::Timestamp,
    DomainError,
};
use serde_json::Value;
use tokio::time::Instant;

use crate::{
    extension_job_authority::{JobAuthorityError, JobAuthorityHost, JobExecution},
    extension_job_operations::{JobWork, JobWorkOutcome, RegisteredJobOperation},
};

const CLAIM_LEASE_MS: i64 = 30_000;
const RENEW_EVERY: Duration = Duration::from_secs(10);
const CANCEL_GRACE: Duration = Duration::from_secs(5);
const POLL_EVERY: Duration = Duration::from_millis(250);
const MAX_CAS_RETRIES: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("extension job storage failed: {0}")]
    Store(#[from] ExtensionJobStoreError),
    #[error("extension job database failed: {0}")]
    Database(#[from] DomainError),
    #[error(transparent)]
    Authority(#[from] JobAuthorityError),
    #[error("extension job data is invalid")]
    InvalidData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    Completed,
    Cancelled,
    Interrupted,
    /// Its attempt was cut off and, being `idempotent`, it was queued to
    /// run again.
    Requeued,
    Skipped,
}

/// Bounded restart progress; no request or result payload is exposed.
pub struct JobRecoveryPage {
    pub examined: usize,
    /// Expired attempts that ended their job `interrupted`.
    pub interrupted: usize,
    /// Expired attempts whose `idempotent` job was queued again.
    pub requeued: usize,
    pub next: Option<RecoveryCursor>,
}

/// Where a test can cut a dispatch off, as a crash would: the hook panics
/// (or the persistence boundary it sits in is left unfinished) and the
/// durable state is all that is left.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// The claim is durable; the worker hasn't started.
    AfterClaim,
    /// The worker returned; nothing is published yet.
    BeforePublish,
    /// Inside the publish transaction: the job's end is written, its
    /// `contribution.job.completed` isn't yet.
    BetweenTransitionAndAnnounce,
}

#[cfg(test)]
pub(crate) type CrashHook = Arc<dyn Fn(Stage) + Send + Sync>;

pub struct ExtensionJobDispatcher {
    db: Database,
    store: SqliteExtensionJobStore,
    host: Arc<JobAuthorityHost>,
    vocabulary: Option<oxplow_domain::vocabulary::VocabularyHandle>,
    #[cfg(test)]
    crash: Option<CrashHook>,
}

impl ExtensionJobDispatcher {
    pub fn new(db: Database, host: Arc<JobAuthorityHost>) -> Self {
        Self {
            store: SqliteExtensionJobStore::new(db.clone()),
            db,
            host,
            vocabulary: None,
            #[cfg(test)]
            crash: None,
        }
    }

    /// Call `hook` at each [`Stage`] of a dispatch: a test crashes there.
    #[cfg(test)]
    pub(crate) fn with_crash_hook(mut self, hook: CrashHook) -> Self {
        self.crash = Some(hook);
        self
    }

    #[cfg(test)]
    fn reach(&self, stage: Stage) {
        if let Some(hook) = &self.crash {
            hook(stage);
        }
    }

    pub fn with_vocabulary(
        mut self,
        vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    ) -> Self {
        self.vocabulary = Some(vocabulary);
        self
    }

    /// Fence expired attempts after restart without asserting their workers
    /// stopped. An `idempotent` job is queued again (until `MAX_ATTEMPTS`);
    /// any other ends `interrupted`. Already-interrupted rows advance the
    /// cursor so they cannot starve later expired work.
    pub async fn recover_page(
        &self,
        after: Option<RecoveryCursor>,
        limit: usize,
    ) -> Result<JobRecoveryPage, DispatchError> {
        self.host
            .read_generation(self.recover_page_unpinned(after, limit))
            .await?
    }

    async fn recover_page_unpinned(
        &self,
        after: Option<RecoveryCursor>,
        limit: usize,
    ) -> Result<JobRecoveryPage, DispatchError> {
        let page = self
            .store
            .recovery_candidates(now_ms(), after, limit)
            .await?;
        let (mut interrupted, mut requeued) = (0, 0);
        for candidate in &page.items {
            if candidate.job.state != JobState::Running {
                continue;
            }
            match self
                .transition_announced(
                    candidate.job.id.clone(),
                    candidate.revision,
                    JobEvent::RecoverExpired { now_ms: now_ms() },
                    None,
                )
                .await
            {
                Ok(recovered) if recovered.job.state == JobState::Queued => requeued += 1,
                Ok(_) => interrupted += 1,
                Err(
                    ExtensionJobStoreError::Conflict
                    | ExtensionJobStoreError::TransitionRejected(_),
                ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(JobRecoveryPage {
            examined: page.items.len(),
            interrupted,
            requeued,
            next: page.next,
        })
    }

    /// Claim and run at most one queued job. No scheduler or automatic retry
    /// is implied by this entry point.
    pub async fn run_one(&self, id: JobId) -> Result<DispatchOutcome, DispatchError> {
        // Fresh host readiness precedes operation lookup/claim. The retained
        // reader covers execution and publication; spawned workers receive it
        // through their host-derived RunContext, never persisted provenance.
        self.host.read_generation(self.run_one_unpinned(id)).await?
    }

    async fn run_one_unpinned(&self, id: JobId) -> Result<DispatchOutcome, DispatchError> {
        let stored = self.store.get(id.clone()).await?;
        if stored.job.state != JobState::Queued || stored.job.cancellation_requested_at_ms.is_some()
        {
            return Ok(if stored.job.state == JobState::Cancelled {
                DispatchOutcome::Cancelled
            } else {
                DispatchOutcome::Skipped
            });
        }
        let context = match self.store.get_context(id.clone()).await {
            Ok(context) => context,
            Err(ExtensionJobStoreError::MissingContext | ExtensionJobStoreError::InvalidData) => {
                return Ok(DispatchOutcome::Skipped);
            }
            Err(error) => return Err(error.into()),
        };
        let registration = match self
            .host
            .registry()
            .find(&stored.job.owner, &stored.operation)
        {
            Ok(registration) if registration.replay_policy() == stored.job.replay_policy => {
                registration
            }
            _ => return Ok(DispatchOutcome::Skipped),
        };

        // Derive the would-be token from the pure transition and authorize it
        // before the durable claim. Claim itself remains a revision CAS.
        let claim_time = now_ms();
        let prospective = stored
            .job
            .transition(JobEvent::Claim {
                now_ms: claim_time,
                lease_ms: CLAIM_LEASE_MS,
                intent_key: stored.job.intent_key.clone(),
            })
            .map_err(|_| DispatchError::InvalidData)?;
        let token = prospective.token.ok_or(DispatchError::InvalidData)?;
        let identity =
            JobExecutionIdentity::new(id.clone(), token).map_err(|_| DispatchError::InvalidData)?;
        match self.host.authorize(&stored, &identity, &context).await {
            Ok(execution) if execution.run().check_authority().is_ok() => {}
            Err(JobAuthorityError::Storage(error)) => return Err(error.into()),
            _ => return Ok(DispatchOutcome::Skipped),
        }

        let claimed = match self
            .transition_announced(
                id.clone(),
                stored.revision,
                JobEvent::Claim {
                    now_ms: now_ms(),
                    lease_ms: CLAIM_LEASE_MS,
                    intent_key: stored.job.intent_key.clone(),
                },
                None,
            )
            .await
        {
            Ok(claimed) => claimed,
            Err(ExtensionJobStoreError::Conflict) => return Ok(DispatchOutcome::Skipped),
            Err(error) => return Err(error.into()),
        };
        let token = AttemptFence {
            attempt: claimed.job.attempt,
            fence: claimed.job.fence,
        };
        if Some(token) != prospective.token {
            return Ok(DispatchOutcome::Interrupted);
        }
        #[cfg(test)]
        self.reach(Stage::AfterClaim);
        let identity =
            JobExecutionIdentity::new(id.clone(), token).map_err(|_| DispatchError::InvalidData)?;

        // The authority that approved the prospective claim is never reused
        // as the authority for the external operation.
        let execution = match self.host.authorize(&claimed, &identity, &context).await {
            Ok(execution) if execution.run().check_authority().is_ok() => execution,
            _ => {
                self.request_cancellation(&id).await?;
                return self.confirm_stopped(&id, token, None).await;
            }
        };
        let work = JobWork::new(
            identity,
            registration.key(),
            execution.run().clone(),
            stored.request,
            context,
        )
        .map_err(|_| DispatchError::InvalidData)?;

        self.supervise(id, token, registration, execution, work)
            .await
    }

    async fn supervise(
        &self,
        id: JobId,
        token: AttemptFence,
        registration: Arc<RegisteredJobOperation>,
        execution: JobExecution,
        work: JobWork,
    ) -> Result<DispatchOutcome, DispatchError> {
        let _cancel_on_drop = CancelOnDrop(execution.run().cancel().clone());
        let mut worker = Box::pin(registration.execute(work));
        let started = Instant::now();
        let mut last_renewal = started;
        let mut cancellation_at: Option<Instant> = None;
        let mut worker_finished = false;

        loop {
            tokio::select! {
                result = &mut worker, if !worker_finished => {
                    worker_finished = true;
                    match result {
                        Ok(JobWorkOutcome::Completed { completion, result }) => {
                            return self.finish_completion(&id, token, execution, completion, result).await;
                        }
                        Ok(JobWorkOutcome::Stopped { result }) => {
                            self.request_cancellation(&id).await?;
                            return self.confirm_stopped(&id, token, result).await;
                        }
                        Ok(JobWorkOutcome::Uncertain) | Err(_) => {
                            self.request_cancellation(&id).await?;
                            execution.run().cancel().cancel();
                            cancellation_at.get_or_insert_with(Instant::now);
                        }
                    }
                }
                _ = tokio::time::sleep(POLL_EVERY) => {}
            }

            let now = Instant::now();
            if !worker_finished {
                let latest = self.store.metadata(id.clone()).await?;
                if latest.job.state != JobState::Running
                    || latest.job.attempt != token.attempt
                    || latest.job.fence != token.fence
                {
                    execution.run().cancel().cancel();
                    return Ok(DispatchOutcome::Interrupted);
                }
                if latest.job.cancellation_requested_at_ms.is_some()
                    || execution.run().check_authority().is_err()
                    || now.duration_since(started) >= registration.max_runtime()
                {
                    if cancellation_at.is_none() {
                        self.request_cancellation(&id).await?;
                        execution.run().cancel().cancel();
                        cancellation_at = Some(now);
                    }
                } else if now.duration_since(last_renewal) >= RENEW_EVERY {
                    match self
                        .transition_retry(
                            &id,
                            JobEvent::Renew {
                                now_ms: now_ms(),
                                lease_ms: CLAIM_LEASE_MS,
                                token,
                            },
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(ExtensionJobStoreError::Conflict)
                        | Err(ExtensionJobStoreError::TransitionRejected(_)) => {
                            self.request_cancellation(&id).await?;
                            execution.run().cancel().cancel();
                            cancellation_at = Some(now);
                        }
                        Err(error) => return Err(error.into()),
                    }
                    last_renewal = now;
                }
            }

            if let Some(cancelled_at) = cancellation_at {
                if now.duration_since(cancelled_at) >= CANCEL_GRACE || worker_finished {
                    drop(worker);
                    return self.recover_after_lease(&id, token).await;
                }
            }
        }
    }

    async fn finish_completion(
        &self,
        id: &JobId,
        token: AttemptFence,
        execution: JobExecution,
        completion: Completion,
        result: Value,
    ) -> Result<DispatchOutcome, DispatchError> {
        if !bounded_json(&result) {
            execution.run().cancel().cancel();
            self.request_cancellation(id).await?;
            return self.recover_after_lease(id, token).await;
        }
        #[cfg(test)]
        self.reach(Stage::BeforePublish);
        // Bounded before duplication: one copy is for the attempted atomic
        // publication, and one only for an append-only stale observation.
        let evidence = result.clone();
        for _ in 0..MAX_CAS_RETRIES {
            let latest = self.store.metadata(id.clone()).await?;
            if latest.job.state != JobState::Running
                || latest.job.attempt != token.attempt
                || latest.job.fence != token.fence
            {
                self.record_late(id, token, completion, evidence).await?;
                return Ok(DispatchOutcome::Interrupted);
            }
            let db = self.db.clone();
            let job_id = id.clone();
            let expected_revision = latest.revision;
            let outcome = completion;
            let result_for_tx = result.clone();
            let savepoint_execution = execution.clone();
            let vocabulary = self.vocabulary.clone();
            #[cfg(test)]
            let crash = self.crash.clone();
            let published = db
                .transaction_or(move |tx| {
                    tx.execute_batch("SAVEPOINT extension_job_publication")
                        .map_err(|error| TxError::Storage(map_sql_err(error)))?;
                    let publish_result = (|| {
                        savepoint_execution
                            .check_publication(tx)
                            .map_err(publication_failure)?;
                        let completed = transition_tx(
                            tx,
                            &job_id,
                            expected_revision,
                            &JobEvent::Complete {
                                now_ms: now_ms(),
                                token,
                                outcome,
                            },
                            Some(&result_for_tx),
                        )
                        .map_err(PublishFailure::Store)?;
                        #[cfg(test)]
                        if let Some(hook) = &crash {
                            hook(Stage::BetweenTransitionAndAnnounce);
                        }
                        if let Some(vocabulary) = &vocabulary {
                            crate::extension_jobs::announce_tx(
                                tx,
                                &vocabulary.current(),
                                &completed.job,
                            )
                            .map_err(PublishFailure::Storage)?;
                        }
                        savepoint_execution
                            .check_publication(tx)
                            .map_err(publication_failure)?;
                        Ok::<_, PublishFailure>(())
                    })();
                    match publish_result {
                        Ok(()) => {
                            tx.execute_batch("RELEASE SAVEPOINT extension_job_publication")
                                .map_err(|error| TxError::Storage(map_sql_err(error)))?;
                            Ok(())
                        }
                        Err(failure) => {
                            tx.execute_batch(
                                "ROLLBACK TO SAVEPOINT extension_job_publication;
                                 RELEASE SAVEPOINT extension_job_publication",
                            )
                            .map_err(|error| TxError::Storage(map_sql_err(error)))?;
                            match failure {
                                PublishFailure::Storage(error) => Err(TxError::Storage(error)),
                                PublishFailure::Revoked => {
                                    Err(TxError::Aborted(PublishAbort::Revoked))
                                }
                                PublishFailure::Store(ExtensionJobStoreError::Storage(error)) => {
                                    Err(TxError::Storage(error))
                                }
                                PublishFailure::Store(error) => {
                                    Err(TxError::Aborted(PublishAbort::Store(error)))
                                }
                            }
                        }
                    }
                })
                .await;
            match published {
                Ok(()) => return Ok(DispatchOutcome::Completed),
                Err(TxError::Aborted(PublishAbort::Revoked)) => {
                    // The worker has returned, so it is known to be stopped:
                    // the job ends cancelled at once, never completed under
                    // the authority that is gone, keeping what it had (its
                    // result as a stopped one). Waiting out the lease would
                    // end it `interrupted` for work that is not in doubt.
                    let mut partial = result.clone();
                    if let Some(parts) = partial.as_object_mut() {
                        parts.insert("stopped".into(), Value::from("cancelled"));
                    }
                    self.record_late(id, token, completion, evidence).await?;
                    self.request_cancellation(id).await?;
                    execution.run().cancel().cancel();
                    return self.confirm_stopped(id, token, Some(partial)).await;
                }
                Err(TxError::Aborted(PublishAbort::Store(ExtensionJobStoreError::Conflict))) => {
                    continue;
                }
                Err(TxError::Aborted(PublishAbort::Store(
                    ExtensionJobStoreError::TransitionRejected(_),
                ))) => {
                    self.record_late(id, token, completion, evidence).await?;
                    return Ok(DispatchOutcome::Interrupted);
                }
                Err(TxError::Aborted(PublishAbort::Store(error))) => return Err(error.into()),
                Err(TxError::Storage(error)) => return Err(error.into()),
            }
        }
        self.record_late(id, token, completion, evidence).await?;
        Ok(DispatchOutcome::Interrupted)
    }

    async fn confirm_stopped(
        &self,
        id: &JobId,
        token: AttemptFence,
        result: Option<Value>,
    ) -> Result<DispatchOutcome, DispatchError> {
        let result = result.filter(bounded_json);
        match self
            .transition_result_retry(
                id,
                JobEvent::ConfirmStopped {
                    now_ms: now_ms(),
                    token,
                },
                result,
            )
            .await
        {
            Ok(stopped) if stopped.job.state == JobState::Cancelled => {
                Ok(DispatchOutcome::Cancelled)
            }
            Ok(_) => Ok(DispatchOutcome::Interrupted),
            Err(ExtensionJobStoreError::Conflict)
            | Err(ExtensionJobStoreError::TransitionRejected(_)) => {
                self.recover_after_lease(id, token).await
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn request_cancellation(&self, id: &JobId) -> Result<(), DispatchError> {
        for _ in 0..MAX_CAS_RETRIES {
            let current = self.store.metadata(id.clone()).await?;
            if current.job.cancellation_requested_at_ms.is_some()
                || matches!(
                    current.job.state,
                    JobState::Succeeded | JobState::Failed | JobState::Cancelled
                )
            {
                return Ok(());
            }
            match self
                .transition_announced(
                    id.clone(),
                    current.revision,
                    JobEvent::RequestCancellation { now_ms: now_ms() },
                    None,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(ExtensionJobStoreError::Conflict)
                | Err(ExtensionJobStoreError::TransitionRejected(
                    oxplow_domain::extension_job::TransitionError::InvalidTransition,
                )) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(ExtensionJobStoreError::Conflict.into())
    }

    async fn recover_after_lease(
        &self,
        id: &JobId,
        token: AttemptFence,
    ) -> Result<DispatchOutcome, DispatchError> {
        let deadline = Instant::now() + Duration::from_millis(CLAIM_LEASE_MS as u64);
        loop {
            let latest = self.store.metadata(id.clone()).await?;
            if latest.job.state != JobState::Running
                || latest.job.attempt != token.attempt
                || latest.job.fence != token.fence
            {
                return Ok(if latest.job.state == JobState::Cancelled {
                    DispatchOutcome::Cancelled
                } else {
                    DispatchOutcome::Interrupted
                });
            }
            let lease_until = latest
                .job
                .lease_until_ms
                .ok_or(DispatchError::InvalidData)?;
            let now = now_ms();
            if now >= lease_until {
                match self
                    .transition_retry(id, JobEvent::RecoverExpired { now_ms: now })
                    .await
                {
                    Ok(recovered) if recovered.job.state == JobState::Queued => {
                        return Ok(DispatchOutcome::Requeued)
                    }
                    Ok(_) => return Ok(DispatchOutcome::Interrupted),
                    Err(ExtensionJobStoreError::Conflict)
                    | Err(ExtensionJobStoreError::TransitionRejected(_)) => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if Instant::now() >= deadline {
                // A clock rollback or another host's lease extension cannot
                // keep a dispatcher slot forever. Persisted recovery waits
                // for actual expiry; no stop/completion is invented here.
                return Ok(DispatchOutcome::Interrupted);
            }
            tokio::time::sleep(Duration::from_millis((lease_until - now).min(250) as u64)).await;
        }
    }

    /// A transition, and when it ends the job, its
    /// `contribution.job.completed` — in one transaction. An expired
    /// attempt's `idempotent` job is queued again in the same transaction
    /// (`requeue_tx`), so it is never seen `interrupted` and nothing is
    /// announced until its real end.
    async fn transition_announced(
        &self,
        id: JobId,
        expected_revision: u64,
        event: JobEvent,
        result: Option<Value>,
    ) -> Result<StoredExtensionJob, ExtensionJobStoreError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction_or(move |tx| {
                let aborted = |e| match e {
                    ExtensionJobStoreError::Storage(storage) => TxError::Storage(storage),
                    other => TxError::Aborted(other),
                };
                let mut changed =
                    transition_tx(tx, &id, expected_revision, &event, result.as_ref())
                        .map_err(aborted)?;
                if matches!(event, JobEvent::RecoverExpired { .. }) {
                    changed = requeue_tx(tx, changed).map_err(aborted)?;
                }
                if let Some(vocabulary) = &vocabulary {
                    crate::extension_jobs::announce_tx(tx, &vocabulary.current(), &changed.job)
                        .map_err(TxError::Storage)?;
                }
                Ok(changed)
            })
            .await
            .map_err(|e| match e {
                TxError::Storage(storage) => ExtensionJobStoreError::Storage(storage),
                TxError::Aborted(reason) => reason,
            })
    }

    async fn record_late(
        &self,
        id: &JobId,
        token: AttemptFence,
        outcome: Completion,
        evidence: Value,
    ) -> Result<(), DispatchError> {
        self.store
            .record_observation(
                id.clone(),
                LateOutcomeObservation {
                    id: JobId::generate().as_str().to_owned(),
                    token,
                    observed_at_ms: now_ms(),
                    outcome,
                    evidence: Some(evidence),
                },
            )
            .await?;
        Ok(())
    }

    async fn transition_retry(
        &self,
        id: &JobId,
        event: JobEvent,
    ) -> Result<StoredExtensionJob, ExtensionJobStoreError> {
        for _ in 0..MAX_CAS_RETRIES {
            let latest = self.store.metadata(id.clone()).await?;
            match self
                .transition_announced(
                    id.clone(),
                    latest.revision,
                    event_at(event.clone(), now_ms()),
                    None,
                )
                .await
            {
                Ok(updated) => return Ok(updated),
                Err(ExtensionJobStoreError::Conflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(ExtensionJobStoreError::Conflict)
    }

    async fn transition_result_retry(
        &self,
        id: &JobId,
        event: JobEvent,
        result: Option<Value>,
    ) -> Result<StoredExtensionJob, ExtensionJobStoreError> {
        for _ in 0..MAX_CAS_RETRIES {
            let latest = self.store.metadata(id.clone()).await?;
            match self
                .transition_announced(
                    id.clone(),
                    latest.revision,
                    event_at(event.clone(), now_ms()),
                    result.clone(),
                )
                .await
            {
                Ok(updated) => return Ok(updated),
                Err(ExtensionJobStoreError::Conflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(ExtensionJobStoreError::Conflict)
    }
}

struct CancelOnDrop(oxplow_domain::run::Cancel);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn publication_failure(error: JobAuthorityError) -> PublishFailure {
    match error {
        JobAuthorityError::Storage(error) => PublishFailure::Storage(error),
        _ => PublishFailure::Revoked,
    }
}

enum PublishFailure {
    Revoked,
    Storage(DomainError),
    Store(ExtensionJobStoreError),
}

enum PublishAbort {
    Revoked,
    Store(ExtensionJobStoreError),
}

/// `job`, just interrupted, queued again when its replay policy allows
/// another attempt (`JobEvent::RetryInterrupted`); as it was otherwise.
fn requeue_tx(
    tx: &rusqlite::Connection,
    job: StoredExtensionJob,
) -> Result<StoredExtensionJob, ExtensionJobStoreError> {
    let retry = JobEvent::RetryInterrupted {
        now_ms: job.job.updated_at_ms,
        intent_key: job.job.intent_key.clone(),
    };
    match transition_tx(tx, &job.job.id, job.revision, &retry, None) {
        Ok(requeued) => Ok(requeued),
        Err(ExtensionJobStoreError::TransitionRejected(
            oxplow_domain::extension_job::TransitionError::ReplayNotAllowed,
        )) => Ok(job),
        Err(error) => Err(error),
    }
}

fn now_ms() -> i64 {
    Timestamp::now().unix_ms()
}

fn event_at(mut event: JobEvent, now: i64) -> JobEvent {
    match &mut event {
        JobEvent::Claim { now_ms, .. }
        | JobEvent::Renew { now_ms, .. }
        | JobEvent::Complete { now_ms, .. }
        | JobEvent::RequestCancellation { now_ms }
        | JobEvent::ConfirmStopped { now_ms, .. }
        | JobEvent::RecoverExpired { now_ms }
        | JobEvent::RetryInterrupted { now_ms, .. } => *now_ms = now,
    }
    event
}

fn bounded_json(value: &Value) -> bool {
    let mut lower_bound = 0usize;
    if !json_lower_bound(value, 1, &mut lower_bound) {
        return false;
    }
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > 1_048_576 {
                return Err(std::io::Error::other("size limit"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Counter(0), value).is_ok()
}

fn json_lower_bound(value: &Value, depth: usize, bytes: &mut usize) -> bool {
    if depth > 64 {
        return false;
    }
    let add = |bytes: &mut usize, amount: usize| {
        *bytes = bytes.saturating_add(amount);
        *bytes <= 1_048_576
    };
    match value {
        Value::Null => add(bytes, 4),
        Value::Bool(true) => add(bytes, 4),
        Value::Bool(false) => add(bytes, 5),
        Value::Number(_) => add(bytes, 1),
        Value::String(text) => add(bytes, text.len().saturating_add(2)),
        Value::Array(items) => {
            if !add(bytes, 2usize.saturating_add(items.len().saturating_sub(1))) {
                return false;
            }
            items
                .iter()
                .all(|item| json_lower_bound(item, depth + 1, bytes))
        }
        Value::Object(items) => {
            for (index, (key, item)) in items.iter().enumerate() {
                if !add(
                    bytes,
                    key.len()
                        .saturating_add(3 + usize::from(u8::from(index > 0))),
                ) || !json_lower_bound(item, depth + 1, bytes)
                {
                    return false;
                }
            }
            add(bytes, 2)
        }
    }
}
