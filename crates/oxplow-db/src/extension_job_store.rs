//! Durable host-managed extension jobs and their private attempt history.
//!
//! This store records attribution and lifecycle only; it does not grant the
//! caller authority to dispatch an operation or publish its result.

use std::{io, io::Write};

use oxplow_domain::{
    contribution::ContributionRef,
    extension_job::{
        AttemptFence, Completion, ExtensionJob, IntentKey, JobEvent, JobId, JobOutput,
        JobOutputKind, JobOwner, JobState, JobTransition, NewJobOutput, ReplayPolicy,
        TransitionError,
    },
    extension_job_context::JobInvocationContext,
    run::RunId,
    DomainError,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use crate::{database::map_sql_err, Database, TxError};

const MAX_JSON_BYTES: usize = 1_048_576;
const MAX_JSON_DEPTH: usize = 64;
const MAX_PAGE_ITEMS: usize = 256;
const MAX_PAGE_BYTES: usize = 4 * 1_048_576;

#[derive(Clone)]
pub struct SqliteExtensionJobStore {
    db: Database,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewExtensionJob {
    pub job: ExtensionJob,
    pub operation: String,
    pub invoker: String,
    pub request: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredExtensionJob {
    pub job: ExtensionJob,
    pub operation: String,
    pub invoker: String,
    pub request: Value,
    pub revision: u64,
}

/// A recovery listing deliberately omits the private request payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCandidate {
    pub job: ExtensionJob,
    pub operation: String,
    pub invoker: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueResult {
    Created(StoredExtensionJob),
    Existing(StoredExtensionJob),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptRecord {
    pub job_id: JobId,
    pub token: AttemptFence,
    pub started_at_ms: i64,
    pub lease_until_ms: i64,
    pub closed_at_ms: Option<i64>,
    pub outcome: Option<AttemptOutcome>,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationCursor {
    pub observed_at_ms: i64,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCursor {
    pub updated_at_ms: i64,
    pub id: JobId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LateOutcomeObservation {
    pub id: String,
    pub token: AttemptFence,
    pub observed_at_ms: i64,
    pub outcome: Completion,
    pub evidence: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeObservation {
    pub job_id: JobId,
    pub observation: LateOutcomeObservation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPage<T, C> {
    pub items: Vec<T>,
    pub next: Option<C>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ExtensionJobStoreError {
    #[error("job data is invalid or exceeds storage limits")]
    InvalidData,
    #[error("job invocation context is missing")]
    MissingContext,
    #[error("job writes require a caller-owned transaction")]
    TransactionRequired,
    #[error("intent key is already bound to different immutable job data")]
    IntentConflict,
    #[error("job revision or observation identity conflicts with stored data")]
    Conflict,
    #[error("job was not found")]
    NotFound,
    #[error("job transition was rejected: {0}")]
    TransitionRejected(TransitionError),
    #[error("extension job storage failed: {0}")]
    Storage(DomainError),
}

type StoreResult<T> = Result<T, ExtensionJobStoreError>;

impl SqliteExtensionJobStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn enqueue(&self, new_job: NewExtensionJob) -> StoreResult<EnqueueResult> {
        self.db
            .transaction_or(move |tx| enqueue_tx(tx, &new_job).map_err(store_tx_error))
            .await
            .map_err(map_store_tx_error)
    }

    /// Trusted scheduling persists immutable provenance with its intent. This
    /// backend API grants no execution authority and makes no worker wake.
    pub async fn enqueue_with_context(
        &self,
        new_job: NewExtensionJob,
        context: JobInvocationContext,
    ) -> StoreResult<EnqueueResult> {
        self.db
            .transaction_or(move |tx| {
                enqueue_with_context_tx(tx, &new_job, &context).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    /// [`append_output_tx`] in its own short transaction: a running job
    /// writes its output this way, never holding a transaction while it
    /// waits on a model.
    pub async fn append_output(
        &self,
        id: JobId,
        token: AttemptFence,
        rows: Vec<NewJobOutput>,
    ) -> StoreResult<u64> {
        self.db
            .transaction_or(move |tx| {
                append_output_tx(tx, &id, token, &rows).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn get_context(&self, id: JobId) -> StoreResult<JobInvocationContext> {
        self.db
            .read_or(move |tx| get_context_tx(tx, &id).map_err(store_tx_error))
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn get(&self, id: JobId) -> StoreResult<StoredExtensionJob> {
        self.db
            .read_or(move |tx| get_tx(tx, &id).map_err(store_tx_error))
            .await
            .map_err(map_store_tx_error)?
            .ok_or(ExtensionJobStoreError::NotFound)
    }

    /// Read dispatch metadata without loading the private request payload.
    pub async fn metadata(&self, id: JobId) -> StoreResult<RecoveryCandidate> {
        self.db
            .read_or(move |tx| metadata_tx(tx, &id).map_err(store_tx_error))
            .await
            .map_err(map_store_tx_error)?
            .ok_or(ExtensionJobStoreError::NotFound)
    }

    pub async fn transition(
        &self,
        id: JobId,
        expected_revision: u64,
        event: JobEvent,
        result: Option<Value>,
    ) -> StoreResult<StoredExtensionJob> {
        self.db
            .transaction_or(move |tx| {
                transition_tx(tx, &id, expected_revision, &event, result.as_ref())
                    .map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn attempts(
        &self,
        id: JobId,
        after_attempt: Option<u32>,
        limit: usize,
    ) -> StoreResult<HistoryPage<AttemptRecord, u32>> {
        self.db
            .read_or(move |tx| attempts_tx(tx, &id, after_attempt, limit).map_err(store_tx_error))
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn observations(
        &self,
        id: JobId,
        after: Option<ObservationCursor>,
        limit: usize,
    ) -> StoreResult<HistoryPage<OutcomeObservation, ObservationCursor>> {
        self.db
            .read_or(move |tx| {
                observations_tx(tx, &id, after.as_ref(), limit).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn record_observation(
        &self,
        id: JobId,
        observation: LateOutcomeObservation,
    ) -> StoreResult<OutcomeObservation> {
        self.db
            .transaction_or(move |tx| {
                record_observation_tx(tx, &id, &observation).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    pub async fn recovery_candidates(
        &self,
        now_ms: i64,
        after: Option<RecoveryCursor>,
        limit: usize,
    ) -> StoreResult<HistoryPage<RecoveryCandidate, RecoveryCursor>> {
        self.db
            .read_or(move |tx| {
                recovery_candidates_tx(tx, now_ms, after.as_ref(), limit).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }

    /// Restart also resumes committed work that was never claimed. This
    /// listing omits request data just like recovery, and grants no dispatch.
    pub async fn queued_candidates(
        &self,
        after: Option<RecoveryCursor>,
        limit: usize,
    ) -> StoreResult<HistoryPage<RecoveryCandidate, RecoveryCursor>> {
        self.db
            .read_or(move |tx| {
                queued_candidates_tx(tx, after.as_ref(), limit).map_err(store_tx_error)
            })
            .await
            .map_err(map_store_tx_error)
    }
}

/// Enqueue and provenance share an inner savepoint, so a caught validation or
/// deduplication error cannot leave a dispatchable partial intent behind.
/// Duplicate delivery preserves the original context, including its RunId.
pub fn enqueue_with_context_tx(
    conn: &Connection,
    new_job: &NewExtensionJob,
    context: &JobInvocationContext,
) -> StoreResult<EnqueueResult> {
    ensure_transaction(conn)?;
    context
        .validate()
        .map_err(|_| ExtensionJobStoreError::InvalidData)?;
    if context.originating_run_id() != &new_job.job.run_id {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let encoded = context
        .encode_json()
        .map_err(|_| ExtensionJobStoreError::InvalidData)?;
    with_savepoint(conn, || {
        let outcome = enqueue_tx(conn, new_job)?;
        match &outcome {
            EnqueueResult::Created(stored) => {
                conn.execute(
                    "INSERT INTO extension_job_context
                        (job_id, originating_run_id, context_json) VALUES (?1, ?2, ?3)",
                    params![
                        stored.job.id.as_str(),
                        context.originating_run_id().as_str(),
                        encoded
                    ],
                )
                .map_err(map_sql_err)?;
            }
            EnqueueResult::Existing(stored) => {
                // Never backfill a context-free row from a redelivery: its
                // original provenance is unknown. Run telemetry is no fallback.
                let original = get_context_tx(conn, &stored.job.id)?;
                if !original.same_scheduling_provenance(context) {
                    return Err(ExtensionJobStoreError::IntentConflict);
                }
            }
        }
        Ok(outcome)
    })
}

/// Read only the private context and immutable origin, with stored byte limits
/// checked before allocation. This does not reconstruct a runnable invocation.
pub fn get_context_tx(conn: &Connection, id: &JobId) -> StoreResult<JobInvocationContext> {
    validate_uuid_v7(id.as_str())?;
    let row = conn
        .query_row(
            "SELECT j.run_id, c.originating_run_id, c.context_json
             FROM extension_job j LEFT JOIN extension_job_context c ON c.job_id = j.id
             WHERE j.id = ?1",
            [id.as_str()],
            |row| {
                let origin = read_text(row, 0, 36)?;
                let context = if matches!(row.get_ref(1)?, rusqlite::types::ValueRef::Null) {
                    None
                } else {
                    Some((read_text(row, 1, 36)?, read_text(row, 2, 65_536)?))
                };
                Ok((origin, context))
            },
        )
        .optional()
        .map_err(map_sql_err)?
        .ok_or(ExtensionJobStoreError::NotFound)?;
    validate_uuid_v7(&row.0)?;
    let (origin, encoded) = row.1.ok_or(ExtensionJobStoreError::MissingContext)?;
    if origin != row.0 {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let context = JobInvocationContext::decode_json(&encoded)
        .map_err(|_| ExtensionJobStoreError::InvalidData)?;
    if context.originating_run_id().as_str() != origin
        || context
            .encode_json()
            .map_err(|_| ExtensionJobStoreError::InvalidData)?
            != encoded
    {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    Ok(context)
}

/// Enqueue in the caller's transaction. A duplicate key returns the original
/// job only if every immutable field except redelivery RunId and candidate id
/// matches. The original run attribution is retained.
pub fn enqueue_tx(conn: &Connection, new_job: &NewExtensionJob) -> StoreResult<EnqueueResult> {
    ensure_transaction(conn)?;
    validate_new_job(new_job)?;
    if let Some(existing) = get_by_intent_tx(
        conn,
        &new_job.job.owner.contribution.to_string(),
        new_job.job.intent_key.as_str(),
    )? {
        return if duplicate_matches(&existing, new_job)? {
            Ok(EnqueueResult::Existing(existing))
        } else {
            Err(ExtensionJobStoreError::IntentConflict)
        };
    }

    let request_json = canonical_json(&new_job.request)?;
    let job = &new_job.job;
    let (contribution_id, package_version, contract_version, deployment_hash) =
        owner_columns(&job.owner);
    let savepoint = Savepoint::new(conn)?;
    let insert = conn.execute(
        "INSERT INTO extension_job (
            id, contribution_id, package_version, contract_version,
            operation, invoker, run_id, intent_key, replay_policy, state, attempt,
            fence, lease_until_ms, cancellation_requested_at_ms, created_at_ms,
            updated_at_ms, revision, request_json, deployment_hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                   ?12, ?13, ?14, ?15, ?16, 1, ?17, ?18)",
        params![
            job.id.as_str(),
            contribution_id,
            package_version,
            contract_version,
            new_job.operation,
            new_job.invoker,
            job.run_id.as_str(),
            job.intent_key.as_str(),
            replay_policy_name(job.replay_policy),
            state_name(job.state),
            i64::from(job.attempt),
            i64::try_from(job.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?,
            job.lease_until_ms,
            job.cancellation_requested_at_ms,
            job.created_at_ms,
            job.updated_at_ms,
            request_json,
            deployment_hash
        ],
    );
    match insert {
        Ok(_) => {
            savepoint.release()?;
            Ok(EnqueueResult::Created(StoredExtensionJob {
                job: job.clone(),
                operation: new_job.operation.clone(),
                invoker: new_job.invoker.clone(),
                request: new_job.request.clone(),
                revision: 1,
            }))
        }
        Err(error) => {
            savepoint.rollback()?;
            // A competing writer can win between the initial lookup and
            // INSERT. Treat its row by the exact same immutable comparison.
            if is_unique_violation(&error) {
                if let Some(existing) = get_by_intent_tx(
                    conn,
                    &new_job.job.owner.contribution.to_string(),
                    new_job.job.intent_key.as_str(),
                )? {
                    return if duplicate_matches(&existing, new_job)? {
                        Ok(EnqueueResult::Existing(existing))
                    } else {
                        Err(ExtensionJobStoreError::IntentConflict)
                    };
                }
                // A candidate job id cannot alias another intent.
                return Err(ExtensionJobStoreError::Conflict);
            }
            Err(ExtensionJobStoreError::Storage(map_sql_err(error)))
        }
    }
}

pub fn get_tx(conn: &Connection, id: &JobId) -> StoreResult<Option<StoredExtensionJob>> {
    validate_uuid_v7(id.as_str())?;
    let raw = conn
        .query_row(
            "SELECT id, contribution_id, package_version, contract_version,
                    operation, invoker, run_id, intent_key, replay_policy, state, attempt,
                    fence, lease_until_ms, cancellation_requested_at_ms, created_at_ms,
                    updated_at_ms, revision, request_json, deployment_hash
             FROM extension_job WHERE id = ?1",
            [id.as_str()],
            decode_stored_job,
        )
        .optional()
        .map_err(map_sql_err)?;
    raw.map(decode_stored_job_data).transpose()
}

/// Point-read the dispatch metadata without selecting `request_json`.
pub fn metadata_tx(conn: &Connection, id: &JobId) -> StoreResult<Option<RecoveryCandidate>> {
    validate_uuid_v7(id.as_str())?;
    let raw = conn
        .query_row(
            "SELECT j.id, j.contribution_id, j.package_version, j.contract_version,
                    j.run_id, j.intent_key, j.replay_policy, j.state, j.attempt, j.fence,
                    j.lease_until_ms, j.cancellation_requested_at_ms, j.created_at_ms,
                    j.updated_at_ms, j.operation, j.invoker, j.revision, j.deployment_hash
             FROM extension_job j WHERE j.id = ?1",
            [id.as_str()],
            read_recovery_candidate,
        )
        .optional()
        .map_err(map_sql_err)?;
    raw.map(decode_recovery_candidate).transpose()
}

pub fn transition_tx(
    conn: &Connection,
    id: &JobId,
    expected_revision: u64,
    event: &JobEvent,
    result: Option<&Value>,
) -> StoreResult<StoredExtensionJob> {
    ensure_transaction(conn)?;
    if expected_revision == 0 || expected_revision > i64::MAX as u64 {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let mut current = get_tx(conn, id)?.ok_or(ExtensionJobStoreError::NotFound)?;
    if current.revision != expected_revision {
        return Err(ExtensionJobStoreError::Conflict);
    }
    let result_json = match event {
        JobEvent::Complete { .. } | JobEvent::ConfirmStopped { .. } => {
            result.map(canonical_json).transpose()?
        }
        _ if result.is_some() => return Err(ExtensionJobStoreError::InvalidData),
        _ => None,
    };
    let transition = current
        .job
        .transition(event.clone())
        .map_err(ExtensionJobStoreError::TransitionRejected)?;
    let next_revision = expected_revision
        .checked_add(1)
        .filter(|revision| *revision <= i64::MAX as u64)
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    with_savepoint(conn, || {
        apply_attempt_change(
            conn,
            &current.job,
            &transition,
            event,
            result_json.as_deref(),
        )?;
        let next = &transition.job;
        let changed = conn
            .execute(
                "UPDATE extension_job SET state=?1, attempt=?2, fence=?3, lease_until_ms=?4,
                cancellation_requested_at_ms=?5, updated_at_ms=?6, revision=?7,
                package_version=?16, deployment_hash=?17
         WHERE id=?8 AND revision=?9 AND state=?10 AND attempt=?11 AND fence=?12
             AND lease_until_ms IS ?13 AND cancellation_requested_at_ms IS ?14
             AND updated_at_ms=?15",
                params![
                    state_name(next.state),
                    i64::from(next.attempt),
                    i64::try_from(next.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?,
                    next.lease_until_ms,
                    next.cancellation_requested_at_ms,
                    next.updated_at_ms,
                    next_revision as i64,
                    id.as_str(),
                    expected_revision as i64,
                    state_name(current.job.state),
                    i64::from(current.job.attempt),
                    i64::try_from(current.job.fence)
                        .map_err(|_| ExtensionJobStoreError::InvalidData)?,
                    current.job.lease_until_ms,
                    current.job.cancellation_requested_at_ms,
                    current.job.updated_at_ms,
                    next.owner.package_version,
                    next.owner.deployment,
                ],
            )
            .map_err(map_sql_err)?;
        if changed != 1 {
            return Err(ExtensionJobStoreError::Conflict);
        }
        current.job = next.clone();
        current.revision = next_revision;
        Ok(current)
    })
}

/// Most bytes one output row keeps (the column's own limit).
pub const MAX_OUTPUT_BYTES: usize = 262_144;

/// Append `rows` to an open attempt's output, after what it has; their
/// sequence numbers follow on. Refused once the attempt is closed (the
/// table's own trigger), so a worker that lost its attempt adds nothing.
pub fn append_output_tx(
    conn: &Connection,
    job_id: &JobId,
    token: AttemptFence,
    rows: &[NewJobOutput],
) -> StoreResult<u64> {
    ensure_transaction(conn)?;
    let fence = i64::try_from(token.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?;
    let mut seq: i64 = conn
        .query_row(
            "SELECT COALESCE(max(seq), 0) FROM extension_job_output
              WHERE job_id = ?1 AND attempt = ?2",
            params![job_id.as_str(), i64::from(token.attempt)],
            |row| row.get(0),
        )
        .map_err(map_sql_err)?;
    let mut insert = conn
        .prepare_cached(
            "INSERT INTO extension_job_output
                (job_id, attempt, fence, seq, kind, at_ms, content_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(map_sql_err)?;
    for row in rows {
        let content =
            serde_json::to_string(&row.content).map_err(|_| ExtensionJobStoreError::InvalidData)?;
        if content.len() > MAX_OUTPUT_BYTES || row.at_ms < 0 {
            return Err(ExtensionJobStoreError::InvalidData);
        }
        seq += 1;
        insert
            .execute(params![
                job_id.as_str(),
                i64::from(token.attempt),
                fence,
                seq,
                row.kind.as_str(),
                row.at_ms,
                content
            ])
            .map_err(map_sql_err)?;
    }
    Ok(u64::try_from(seq).unwrap_or(0))
}

/// An attempt's output after `after_seq`, in order, at most `limit` rows.
pub fn output_tx(
    conn: &Connection,
    job_id: &JobId,
    attempt: u32,
    after_seq: u64,
    limit: usize,
) -> StoreResult<Vec<JobOutput>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT seq, kind, at_ms, content_json FROM extension_job_output
              WHERE job_id = ?1 AND attempt = ?2 AND seq > ?3
              ORDER BY seq LIMIT ?4",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(
            params![
                job_id.as_str(),
                i64::from(attempt),
                i64::try_from(after_seq).map_err(|_| ExtensionJobStoreError::InvalidData)?,
                i64::try_from(limit.min(MAX_PAGE_ITEMS * 16)).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(map_sql_err)?;
    rows.map(|row| {
        let (seq, kind, at_ms, content) = row.map_err(map_sql_err)?;
        Ok(JobOutput {
            seq: u64::try_from(seq).map_err(|_| ExtensionJobStoreError::InvalidData)?,
            kind: JobOutputKind::parse(&kind).ok_or(ExtensionJobStoreError::InvalidData)?,
            at_ms,
            content: serde_json::from_str(&content)
                .map_err(|_| ExtensionJobStoreError::InvalidData)?,
        })
    })
    .collect()
}

pub fn record_observation_tx(
    conn: &Connection,
    job_id: &JobId,
    observation: &LateOutcomeObservation,
) -> StoreResult<OutcomeObservation> {
    ensure_transaction(conn)?;
    validate_uuid_v7(job_id.as_str())?;
    validate_uuid_v7(&observation.id)?;
    if observation.observed_at_ms < 0
        || observation.token.attempt == 0
        || observation.token.fence == 0
        || observation.token.fence != 2 * u64::from(observation.token.attempt) - 1
    {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let evidence_json = observation
        .evidence
        .as_ref()
        .map(canonical_json)
        .transpose()?;
    let attempt_start: Option<i64> = conn
        .query_row(
            "SELECT started_at_ms FROM extension_job_attempt
         WHERE job_id=?1 AND attempt=?2 AND fence=?3",
            params![
                job_id.as_str(),
                i64::from(observation.token.attempt),
                i64::try_from(observation.token.fence)
                    .map_err(|_| ExtensionJobStoreError::InvalidData)?
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    if attempt_start.is_none_or(|started| observation.observed_at_ms < started) {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let outcome = completion_name(observation.outcome);
    let existing: Option<(String, i64, i64, i64, String, Option<String>)> = conn
        .query_row(
            "SELECT job_id, attempt, fence, observed_at_ms, outcome, evidence_json
         FROM extension_job_observation WHERE id=?1",
            [&observation.id],
            |row| {
                Ok((
                    read_text(row, 0, 36)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    read_text(row, 4, 9)?,
                    read_optional_text(row, 5, 1048576)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?;
    if let Some((existing_job, attempt, fence, time, existing_outcome, existing_evidence)) =
        existing
    {
        if existing_job == job_id.as_str()
            && attempt == i64::from(observation.token.attempt)
            && u64::try_from(fence).ok() == Some(observation.token.fence)
            && time == observation.observed_at_ms
            && existing_outcome == outcome
            && existing_evidence == evidence_json
        {
            return Ok(OutcomeObservation {
                job_id: job_id.clone(),
                observation: observation.clone(),
            });
        }
        return Err(ExtensionJobStoreError::Conflict);
    }
    conn.execute(
        "INSERT INTO extension_job_observation (id, job_id, attempt, fence, observed_at_ms, outcome, evidence_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![observation.id, job_id.as_str(), i64::from(observation.token.attempt),
            i64::try_from(observation.token.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?,
            observation.observed_at_ms, outcome, evidence_json],
    ).map_err(map_sql_err)?;
    Ok(OutcomeObservation {
        job_id: job_id.clone(),
        observation: observation.clone(),
    })
}

pub fn attempts_tx(
    conn: &Connection,
    job_id: &JobId,
    after_attempt: Option<u32>,
    limit: usize,
) -> StoreResult<HistoryPage<AttemptRecord, u32>> {
    validate_page(limit)?;
    validate_uuid_v7(job_id.as_str())?;
    let after = after_attempt.unwrap_or(0);
    let mut statement = conn.prepare(
        "SELECT job_id, attempt, fence, started_at_ms, lease_until_ms, closed_at_ms, outcome, result_json
         FROM extension_job_attempt WHERE job_id=?1 AND attempt>?2 ORDER BY attempt LIMIT ?3",
    ).map_err(map_sql_err)?;
    let rows = statement
        .query_map(
            params![job_id.as_str(), i64::from(after), (limit + 1) as i64],
            read_attempt,
        )
        .map_err(map_sql_err)?;
    let mut items = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    for row in rows {
        if items.len() == limit {
            more = true;
            break;
        }
        let raw = row.map_err(map_sql_err)?;
        let size = raw.7.as_ref().map_or(0, String::len);
        if size > MAX_JSON_BYTES {
            return Err(ExtensionJobStoreError::InvalidData);
        }
        if bytes + size > MAX_PAGE_BYTES {
            more = true;
            break;
        }
        let item = decode_attempt(raw)?;
        bytes += size;
        items.push(item);
    }
    let next = more
        .then(|| items.last().map(|item| item.token.attempt))
        .flatten();
    Ok(HistoryPage { items, next })
}

pub fn observations_tx(
    conn: &Connection,
    job_id: &JobId,
    after: Option<&ObservationCursor>,
    limit: usize,
) -> StoreResult<HistoryPage<OutcomeObservation, ObservationCursor>> {
    validate_page(limit)?;
    validate_uuid_v7(job_id.as_str())?;
    let (after_time, after_id) = after.map_or((-1, String::new()), |cursor| {
        (cursor.observed_at_ms, cursor.id.clone())
    });
    if after
        .is_some_and(|cursor| cursor.observed_at_ms < 0 || validate_uuid_v7(&cursor.id).is_err())
    {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let mut statement = conn
        .prepare(
            "SELECT id, attempt, fence, observed_at_ms, outcome, evidence_json
         FROM extension_job_observation WHERE job_id=?1
           AND (observed_at_ms>?2 OR (observed_at_ms=?2 AND id>?3))
         ORDER BY observed_at_ms, id LIMIT ?4",
        )
        .map_err(map_sql_err)?;
    let rows = statement
        .query_map(
            params![job_id.as_str(), after_time, after_id, (limit + 1) as i64],
            read_observation,
        )
        .map_err(map_sql_err)?;
    let mut items = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    for row in rows {
        if items.len() == limit {
            more = true;
            break;
        }
        let raw = row.map_err(map_sql_err)?;
        let size = raw.5.as_ref().map_or(0, String::len);
        if size > MAX_JSON_BYTES {
            return Err(ExtensionJobStoreError::InvalidData);
        }
        if bytes + size > MAX_PAGE_BYTES {
            more = true;
            break;
        }
        let item = decode_observation(raw, job_id)?;
        bytes += size;
        items.push(item);
    }
    let next = more
        .then(|| {
            items.last().map(|item| ObservationCursor {
                observed_at_ms: item.observation.observed_at_ms,
                id: item.observation.id.clone(),
            })
        })
        .flatten();
    Ok(HistoryPage { items, next })
}

pub fn recovery_candidates_tx(
    conn: &Connection,
    now_ms: i64,
    after: Option<&RecoveryCursor>,
    limit: usize,
) -> StoreResult<HistoryPage<RecoveryCandidate, RecoveryCursor>> {
    candidates_tx(conn, now_ms, after, limit, false)
}

pub fn queued_candidates_tx(
    conn: &Connection,
    after: Option<&RecoveryCursor>,
    limit: usize,
) -> StoreResult<HistoryPage<RecoveryCandidate, RecoveryCursor>> {
    candidates_tx(conn, 0, after, limit, true)
}

fn candidates_tx(
    conn: &Connection,
    now_ms: i64,
    after: Option<&RecoveryCursor>,
    limit: usize,
    queued: bool,
) -> StoreResult<HistoryPage<RecoveryCandidate, RecoveryCursor>> {
    validate_page(limit)?;
    if now_ms < 0 {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    if let Some(cursor) = after {
        if cursor.updated_at_ms < 0 {
            return Err(ExtensionJobStoreError::InvalidData);
        }
        validate_uuid_v7(cursor.id.as_str())?;
    }
    // Only these host constants select the query; no input supplies SQL.
    let predicate = if queued {
        "j.state='queued' AND ?1 >= 0"
    } else {
        "((j.state='running' AND j.lease_until_ms<=?1) OR j.state='interrupted')"
    };
    let sql = format!(
        "SELECT j.id, j.contribution_id, j.package_version, j.contract_version,
                j.run_id, j.intent_key, j.replay_policy, j.state, j.attempt, j.fence,
                j.lease_until_ms, j.cancellation_requested_at_ms, j.created_at_ms,
                j.updated_at_ms, j.operation, j.invoker, j.revision, j.deployment_hash
         FROM extension_job j
         WHERE {predicate}
           AND (j.updated_at_ms>?2 OR (j.updated_at_ms=?2 AND j.id>?3))
         ORDER BY j.updated_at_ms, j.id LIMIT ?4"
    );
    let mut statement = conn.prepare(&sql).map_err(map_sql_err)?;
    let rows = statement
        .query_map(
            params![
                now_ms,
                after.map_or(-1, |cursor| cursor.updated_at_ms),
                after.map_or("", |cursor| cursor.id.as_str()),
                (limit + 1) as i64
            ],
            read_recovery_candidate,
        )
        .map_err(map_sql_err)?;
    let mut items = Vec::new();
    let mut more = false;
    for row in rows {
        if items.len() == limit {
            more = true;
            break;
        }
        items.push(decode_recovery_candidate(row.map_err(map_sql_err)?)?);
    }
    let next = more
        .then(|| {
            items.last().map(|item| RecoveryCursor {
                updated_at_ms: item.job.updated_at_ms,
                id: item.job.id.clone(),
            })
        })
        .flatten();
    Ok(HistoryPage { items, next })
}

// -- row decoding and transitions -------------------------------------------------

#[derive(Debug)]
struct RawStoredJob {
    id: String,
    contribution_id: String,
    package_version: Option<String>,
    contract_version: Option<i64>,
    operation: String,
    invoker: String,
    run_id: String,
    intent_key: String,
    replay_policy: String,
    state: String,
    attempt: i64,
    fence: i64,
    lease_until_ms: Option<i64>,
    cancellation_requested_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
    request_json: String,
    deployment_hash: Option<String>,
}

// Check borrowed SQLite bytes before allocating a String or decoding JSON.
fn read_optional_text(
    row: &Row<'_>,
    column: usize,
    limit: usize,
) -> rusqlite::Result<Option<String>> {
    let invalid = || {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(io::Error::other(
                "stored job text is invalid or exceeds limits",
            )),
        )
    };
    match row.get_ref(column)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Text(bytes) => {
            if bytes.len() > limit {
                return Err(invalid());
            }
            std::str::from_utf8(bytes)
                .map(|text| Some(text.to_owned()))
                .map_err(|_| invalid())
        }
        _ => row.get(column),
    }
}

fn read_text(row: &Row<'_>, column: usize, limit: usize) -> rusqlite::Result<String> {
    read_optional_text(row, column, limit)?.ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(column, column.to_string(), rusqlite::types::Type::Null)
    })
}

fn decode_stored_job(row: &Row<'_>) -> rusqlite::Result<RawStoredJob> {
    Ok(RawStoredJob {
        id: read_text(row, 0, 36)?,
        contribution_id: read_text(row, 1, 512)?,
        package_version: read_optional_text(row, 2, 256)?,
        contract_version: row.get(3)?,
        operation: read_text(row, 4, 256)?,
        invoker: read_text(row, 5, 256)?,
        run_id: read_text(row, 6, 36)?,
        intent_key: read_text(row, 7, 128)?,
        replay_policy: read_text(row, 8, 10)?,
        state: read_text(row, 9, 11)?,
        attempt: row.get(10)?,
        fence: row.get(11)?,
        lease_until_ms: row.get(12)?,
        cancellation_requested_at_ms: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        revision: row.get(16)?,
        request_json: read_text(row, 17, 1048576)?,
        deployment_hash: read_optional_text(row, 18, 256)?,
    })
}

fn decode_stored_job_data(raw: RawStoredJob) -> StoreResult<StoredExtensionJob> {
    let job = decode_job(JobColumns {
        id: raw.id,
        contribution_id: raw.contribution_id,
        package_version: raw.package_version,
        contract_version: raw.contract_version,
        run_id: raw.run_id,
        intent_key: raw.intent_key,
        replay_policy: raw.replay_policy,
        state: raw.state,
        attempt: raw.attempt,
        fence: raw.fence,
        lease_until_ms: raw.lease_until_ms,
        cancellation_requested_at_ms: raw.cancellation_requested_at_ms,
        created_at_ms: raw.created_at_ms,
        updated_at_ms: raw.updated_at_ms,
        deployment_hash: raw.deployment_hash,
    })?;
    validate_metadata(&raw.operation)?;
    validate_metadata(&raw.invoker)?;
    let revision = u64::try_from(raw.revision)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    let request = parse_bounded_json(&raw.request_json)?;
    Ok(StoredExtensionJob {
        job,
        operation: raw.operation,
        invoker: raw.invoker,
        request,
        revision,
    })
}

struct JobColumns {
    id: String,
    contribution_id: String,
    package_version: Option<String>,
    contract_version: Option<i64>,
    run_id: String,
    intent_key: String,
    replay_policy: String,
    state: String,
    attempt: i64,
    fence: i64,
    lease_until_ms: Option<i64>,
    cancellation_requested_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    deployment_hash: Option<String>,
}

fn decode_job(columns: JobColumns) -> StoreResult<ExtensionJob> {
    let JobColumns {
        id,
        contribution_id,
        package_version,
        contract_version,
        run_id,
        intent_key,
        replay_policy,
        state,
        attempt,
        fence,
        lease_until_ms,
        cancellation_requested_at_ms,
        created_at_ms,
        updated_at_ms,
        deployment_hash,
    } = columns;
    let id = parse_job_id(&id)?;
    let contribution = contribution_id
        .parse::<ContributionRef>()
        .ok()
        .and_then(|r| r.id())
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    let owner = JobOwner {
        contribution,
        package_version,
        contract_version: contract_version
            .map(|v| u32::try_from(v).map_err(|_| ExtensionJobStoreError::InvalidData))
            .transpose()?,
        deployment: deployment_hash,
    };
    let job = ExtensionJob {
        id,
        owner,
        run_id: RunId(run_id),
        intent_key: IntentKey::new(intent_key).map_err(|_| ExtensionJobStoreError::InvalidData)?,
        replay_policy: parse_replay_policy(&replay_policy)?,
        state: parse_state(&state)?,
        attempt: u32::try_from(attempt).map_err(|_| ExtensionJobStoreError::InvalidData)?,
        fence: u64::try_from(fence).map_err(|_| ExtensionJobStoreError::InvalidData)?,
        lease_until_ms,
        cancellation_requested_at_ms,
        created_at_ms,
        updated_at_ms,
    };
    job.validate()
        .map_err(|_| ExtensionJobStoreError::InvalidData)?;
    Ok(job)
}

fn parse_job_id(value: &str) -> StoreResult<JobId> {
    validate_uuid_v7(value)?;
    serde_json::from_value::<JobId>(Value::String(value.to_owned()))
        .map_err(|_| ExtensionJobStoreError::InvalidData)
}

fn apply_attempt_change(
    conn: &Connection,
    old: &ExtensionJob,
    transition: &JobTransition,
    event: &JobEvent,
    result_json: Option<&str>,
) -> StoreResult<()> {
    let new = &transition.job;
    match event {
        JobEvent::Claim { now_ms, .. } => {
            conn.execute(
                "INSERT INTO extension_job_attempt (job_id, attempt, fence, started_at_ms, lease_until_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![old.id.as_str(), i64::from(new.attempt), i64::try_from(new.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?, now_ms, new.lease_until_ms],
            ).map_err(map_sql_err)?;
        }
        JobEvent::Renew { .. } => {
            let changed = conn.execute(
                "UPDATE extension_job_attempt SET lease_until_ms=?1
                 WHERE job_id=?2 AND attempt=?3 AND fence=?4 AND closed_at_ms IS NULL AND lease_until_ms IS ?5",
                params![new.lease_until_ms, old.id.as_str(), i64::from(old.attempt), i64::try_from(old.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?, old.lease_until_ms],
            ).map_err(map_sql_err)?;
            if changed != 1 {
                return Err(ExtensionJobStoreError::Conflict);
            }
        }
        JobEvent::Complete {
            now_ms, outcome, ..
        } => close_attempt(conn, old, *now_ms, completion_name(*outcome), result_json)?,
        JobEvent::ConfirmStopped { now_ms, .. } => {
            close_attempt(conn, old, *now_ms, "cancelled", result_json)?
        }
        JobEvent::RecoverExpired { now_ms } => {
            close_attempt(conn, old, *now_ms, "interrupted", None)?
        }
        JobEvent::RequestCancellation { .. }
        | JobEvent::RetryInterrupted { .. }
        | JobEvent::Rebind { .. } => {}
    }
    Ok(())
}

fn close_attempt(
    conn: &Connection,
    job: &ExtensionJob,
    now_ms: i64,
    outcome: &str,
    result: Option<&str>,
) -> StoreResult<()> {
    let changed = conn
        .execute(
            "UPDATE extension_job_attempt SET closed_at_ms=?1, outcome=?2, result_json=?3
         WHERE job_id=?4 AND attempt=?5 AND fence=?6 AND closed_at_ms IS NULL
             AND lease_until_ms IS ?7",
            params![
                now_ms,
                outcome,
                result,
                job.id.as_str(),
                i64::from(job.attempt),
                i64::try_from(job.fence).map_err(|_| ExtensionJobStoreError::InvalidData)?,
                job.lease_until_ms
            ],
        )
        .map_err(map_sql_err)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(ExtensionJobStoreError::Conflict)
    }
}

type RawAttempt = (
    String,
    i64,
    i64,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn read_attempt(row: &Row<'_>) -> rusqlite::Result<RawAttempt> {
    Ok((
        read_text(row, 0, 36)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        read_optional_text(row, 6, 11)?,
        read_optional_text(row, 7, 1048576)?,
    ))
}

fn decode_attempt(raw: RawAttempt) -> StoreResult<AttemptRecord> {
    let (job_id, attempt, fence, started, lease, closed, outcome, result) = raw;
    let job_id = parse_job_id(&job_id)?;
    let attempt = u32::try_from(attempt)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    let fence = u64::try_from(fence)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    let outcome = match outcome.as_deref() {
        None => None,
        Some(value) => {
            Some(parse_attempt_outcome(value).ok_or(ExtensionJobStoreError::InvalidData)?)
        }
    };
    let result = result.as_deref().map(parse_bounded_json).transpose()?;
    if fence != 2 * u64::from(attempt) - 1
        || started < 0
        || lease <= started
        || closed.is_some_and(|time| time < started)
        || closed.is_some() != outcome.is_some()
        || (result.is_some() && (outcome.is_none() || outcome == Some(AttemptOutcome::Interrupted)))
    {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    Ok(AttemptRecord {
        job_id,
        token: AttemptFence { attempt, fence },
        started_at_ms: started,
        lease_until_ms: lease,
        closed_at_ms: closed,
        outcome,
        result,
    })
}

type RawObservation = (String, i64, i64, i64, String, Option<String>);

fn read_observation(row: &Row<'_>) -> rusqlite::Result<RawObservation> {
    Ok((
        read_text(row, 0, 36)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        read_text(row, 4, 9)?,
        read_optional_text(row, 5, 1048576)?,
    ))
}

fn decode_observation(raw: RawObservation, job_id: &JobId) -> StoreResult<OutcomeObservation> {
    let (id, attempt, fence, time, outcome, evidence) = raw;
    let token = AttemptFence {
        attempt: u32::try_from(attempt)
            .ok()
            .filter(|value| *value > 0)
            .ok_or(ExtensionJobStoreError::InvalidData)?,
        fence: u64::try_from(fence)
            .ok()
            .filter(|value| *value > 0)
            .ok_or(ExtensionJobStoreError::InvalidData)?,
    };
    validate_uuid_v7(&id)?;
    if token.fence != 2 * u64::from(token.attempt) - 1 {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let outcome = match outcome.as_str() {
        "succeeded" => Completion::Succeeded,
        "failed" => Completion::Failed,
        _ => return Err(ExtensionJobStoreError::InvalidData),
    };
    if time < 0 {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let evidence = evidence.as_deref().map(parse_bounded_json).transpose()?;
    Ok(OutcomeObservation {
        job_id: job_id.clone(),
        observation: LateOutcomeObservation {
            id,
            token,
            observed_at_ms: time,
            outcome,
            evidence,
        },
    })
}

type RawRecovery = (
    String,
    String,
    Option<String>,
    Option<i64>,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
    String,
    String,
    i64,
    Option<String>,
);

fn read_recovery_candidate(row: &Row<'_>) -> rusqlite::Result<RawRecovery> {
    Ok((
        read_text(row, 0, 36)?,
        read_text(row, 1, 512)?,
        read_optional_text(row, 2, 256)?,
        row.get(3)?,
        read_text(row, 4, 36)?,
        read_text(row, 5, 128)?,
        read_text(row, 6, 10)?,
        read_text(row, 7, 11)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        read_text(row, 14, 256)?,
        read_text(row, 15, 256)?,
        row.get(16)?,
        read_optional_text(row, 17, 256)?,
    ))
}

fn decode_recovery_candidate(raw: RawRecovery) -> StoreResult<RecoveryCandidate> {
    let (
        id,
        contribution,
        version,
        contract,
        run,
        intent,
        policy,
        state,
        attempt,
        fence,
        lease,
        cancellation,
        created,
        updated,
        operation,
        invoker,
        revision,
        deployment_hash,
    ) = raw;
    let job = decode_job(JobColumns {
        id,
        contribution_id: contribution,
        package_version: version,
        contract_version: contract,
        run_id: run,
        intent_key: intent,
        replay_policy: policy,
        state,
        attempt,
        fence,
        lease_until_ms: lease,
        cancellation_requested_at_ms: cancellation,
        created_at_ms: created,
        updated_at_ms: updated,
        deployment_hash,
    })?;
    validate_metadata(&operation)?;
    validate_metadata(&invoker)?;
    Ok(RecoveryCandidate {
        job,
        operation,
        invoker,
        revision: u64::try_from(revision)
            .ok()
            .filter(|revision| *revision > 0)
            .ok_or(ExtensionJobStoreError::InvalidData)?,
    })
}

// -- validation, canonical JSON and names ----------------------------------------

fn validate_new_job(new: &NewExtensionJob) -> StoreResult<()> {
    new.job
        .validate()
        .map_err(|_| ExtensionJobStoreError::InvalidData)?;
    if new.job.state != JobState::Queued
        || new.job.attempt != 0
        || new.job.fence != 0
        || new.job.lease_until_ms.is_some()
        || new.job.cancellation_requested_at_ms.is_some()
        || new.job.updated_at_ms != new.job.created_at_ms
    {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    validate_metadata(&new.operation)?;
    validate_metadata(&new.invoker)?;
    let _ = canonical_json(&new.request)?;
    validate_uuid_v7(new.job.id.as_str())?;
    validate_uuid_v7(new.job.run_id.as_str())?;
    Ok(())
}

fn validate_metadata(value: &str) -> StoreResult<()> {
    if value.is_empty()
        || value.len() > 256
        || !value.is_ascii()
        || !value.bytes().all(|b| b.is_ascii_graphic())
    {
        Err(ExtensionJobStoreError::InvalidData)
    } else {
        Ok(())
    }
}

fn validate_uuid_v7(value: &str) -> StoreResult<()> {
    let uuid = uuid::Uuid::parse_str(value).map_err(|_| ExtensionJobStoreError::InvalidData)?;
    if uuid.get_version_num() == 7
        && uuid.get_variant() == uuid::Variant::RFC4122
        && uuid.to_string() == value
    {
        Ok(())
    } else {
        Err(ExtensionJobStoreError::InvalidData)
    }
}

fn canonical_json(value: &Value) -> StoreResult<String> {
    validate_json_depth(value, 0, &mut 0)?;
    // Size does not depend on key order. Check it before collecting object
    // entries for sorting, so a rejected request cannot allocate an unbounded
    // additional key vector. Depth validation bounds serializer recursion.
    let mut count = CountWriter(0);
    serde_json::to_writer(&mut count, value).map_err(|_| ExtensionJobStoreError::InvalidData)?;
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit: MAX_JSON_BYTES,
    };
    write_canonical(value, &mut writer).map_err(|_| ExtensionJobStoreError::InvalidData)?;
    String::from_utf8(writer.bytes).map_err(|_| ExtensionJobStoreError::InvalidData)
}

fn write_canonical(value: &Value, out: &mut impl Write) -> io::Result<()> {
    match value {
        Value::Object(map) => {
            out.write_all(b"{")?;
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.write_all(b",")?;
                }
                serde_json::to_writer(&mut *out, key)?;
                out.write_all(b":")?;
                write_canonical(value, out)?;
            }
            out.write_all(b"}")
        }
        Value::Array(items) => {
            out.write_all(b"[")?;
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.write_all(b",")?;
                }
                write_canonical(item, out)?;
            }
            out.write_all(b"]")
        }
        scalar => {
            serde_json::to_writer(out, scalar)?;
            Ok(())
        }
    }
}

fn validate_json_depth(value: &Value, depth: usize, lower_bytes: &mut usize) -> StoreResult<()> {
    if depth > MAX_JSON_DEPTH {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    // Every value consumes at least one byte; strings and keys consume at
    // least their UTF-8 length. This bounds traversal of huge flat inputs
    // before the exact escaped serialization count.
    *lower_bytes = lower_bytes
        .checked_add(match value {
            Value::String(text) => text.len().saturating_add(2),
            _ => 1,
        })
        .filter(|bytes| *bytes <= MAX_JSON_BYTES)
        .ok_or(ExtensionJobStoreError::InvalidData)?;
    match value {
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| validate_json_depth(item, depth + 1, lower_bytes)),
        Value::Object(map) => map.iter().try_for_each(|(key, item)| {
            *lower_bytes = lower_bytes
                .checked_add(key.len().saturating_add(2))
                .filter(|bytes| *bytes <= MAX_JSON_BYTES)
                .ok_or(ExtensionJobStoreError::InvalidData)?;
            validate_json_depth(item, depth + 1, lower_bytes)
        }),
        _ => Ok(()),
    }
}

fn parse_bounded_json(text: &str) -> StoreResult<Value> {
    if text.len() > MAX_JSON_BYTES {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    let value: Value =
        serde_json::from_str(text).map_err(|_| ExtensionJobStoreError::InvalidData)?;
    let canonical = canonical_json(&value)?;
    if canonical != text {
        return Err(ExtensionJobStoreError::InvalidData);
    }
    Ok(value)
}

struct CountWriter(usize);
impl Write for CountWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(buffer.len())
            .filter(|bytes| *bytes <= MAX_JSON_BYTES)
            .ok_or_else(|| io::Error::other("JSON limit"))?;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(buffer.len()) > self.limit {
            return Err(io::Error::other("JSON limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl From<DomainError> for ExtensionJobStoreError {
    fn from(error: DomainError) -> Self {
        Self::Storage(error)
    }
}

struct Savepoint<'a> {
    conn: &'a Connection,
    active: bool,
}

/// Normal error paths roll back explicitly and propagate rollback failures.
/// Drop below is only a last cleanup attempt during unwinding/storage failure.
fn with_savepoint<T>(conn: &Connection, write: impl FnOnce() -> StoreResult<T>) -> StoreResult<T> {
    let savepoint = Savepoint::new(conn)?;
    match write() {
        Ok(value) => {
            savepoint.release()?;
            Ok(value)
        }
        Err(error) => {
            savepoint.rollback()?;
            Err(error)
        }
    }
}
impl<'a> Savepoint<'a> {
    fn new(conn: &'a Connection) -> StoreResult<Self> {
        conn.execute_batch("SAVEPOINT extension_job_store")
            .map_err(map_sql_err)?;
        Ok(Self { conn, active: true })
    }
    fn release(mut self) -> StoreResult<()> {
        self.conn
            .execute_batch("RELEASE extension_job_store")
            .map_err(map_sql_err)?;
        self.active = false;
        Ok(())
    }
    fn rollback(mut self) -> StoreResult<()> {
        self.conn
            .execute_batch("ROLLBACK TO extension_job_store; RELEASE extension_job_store")
            .map_err(map_sql_err)?;
        self.active = false;
        Ok(())
    }
}
impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .conn
                .execute_batch("ROLLBACK TO extension_job_store; RELEASE extension_job_store");
        }
    }
}

fn duplicate_matches(
    existing: &StoredExtensionJob,
    incoming: &NewExtensionJob,
) -> StoreResult<bool> {
    Ok(existing.job.owner == incoming.job.owner
        && existing.job.intent_key == incoming.job.intent_key
        && existing.job.replay_policy == incoming.job.replay_policy
        && existing.operation == incoming.operation
        && existing.invoker == incoming.invoker
        && canonical_json(&existing.request)? == canonical_json(&incoming.request)?)
}

fn get_by_intent_tx(
    conn: &Connection,
    contribution_id: &str,
    intent_key: &str,
) -> StoreResult<Option<StoredExtensionJob>> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM extension_job WHERE contribution_id=?1 AND intent_key=?2",
            params![contribution_id, intent_key],
            |row| read_text(row, 0, 36),
        )
        .optional()
        .map_err(map_sql_err)?;
    id.map(|value| {
        let id = parse_job_id(&value)?;
        get_tx(conn, &id)?.ok_or(ExtensionJobStoreError::NotFound)
    })
    .transpose()
}

fn owner_columns(owner: &JobOwner) -> (String, Option<String>, Option<i64>, Option<String>) {
    (
        owner.contribution.to_string(),
        owner.package_version.clone(),
        owner.contract_version.map(i64::from),
        owner.deployment.clone(),
    )
}

fn replay_policy_name(value: ReplayPolicy) -> &'static str {
    match value {
        ReplayPolicy::Never => "never",
        ReplayPolicy::Idempotent => "idempotent",
    }
}
fn parse_replay_policy(value: &str) -> StoreResult<ReplayPolicy> {
    match value {
        "never" => Ok(ReplayPolicy::Never),
        "idempotent" => Ok(ReplayPolicy::Idempotent),
        _ => Err(ExtensionJobStoreError::InvalidData),
    }
}
fn state_name(value: JobState) -> &'static str {
    match value {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Interrupted => "interrupted",
        JobState::Succeeded => "succeeded",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
    }
}
fn parse_state(value: &str) -> StoreResult<JobState> {
    match value {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running),
        "interrupted" => Ok(JobState::Interrupted),
        "succeeded" => Ok(JobState::Succeeded),
        "failed" => Ok(JobState::Failed),
        "cancelled" => Ok(JobState::Cancelled),
        _ => Err(ExtensionJobStoreError::InvalidData),
    }
}
fn completion_name(value: Completion) -> &'static str {
    match value {
        Completion::Succeeded => "succeeded",
        Completion::Failed => "failed",
    }
}
fn parse_attempt_outcome(value: &str) -> Option<AttemptOutcome> {
    match value {
        "succeeded" => Some(AttemptOutcome::Succeeded),
        "failed" => Some(AttemptOutcome::Failed),
        "cancelled" => Some(AttemptOutcome::Cancelled),
        "interrupted" => Some(AttemptOutcome::Interrupted),
        _ => None,
    }
}
fn validate_page(limit: usize) -> StoreResult<()> {
    if (1..=MAX_PAGE_ITEMS).contains(&limit) {
        Ok(())
    } else {
        Err(ExtensionJobStoreError::InvalidData)
    }
}

fn ensure_transaction(conn: &Connection) -> StoreResult<()> {
    if conn.is_autocommit() {
        Err(ExtensionJobStoreError::TransactionRequired)
    } else {
        Ok(())
    }
}
fn is_unique_violation(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY)
}

fn store_tx_error(error: ExtensionJobStoreError) -> TxError<ExtensionJobStoreError> {
    match error {
        ExtensionJobStoreError::Storage(storage) => TxError::Storage(storage),
        other => TxError::Aborted(other),
    }
}
fn map_store_tx_error(error: TxError<ExtensionJobStoreError>) -> ExtensionJobStoreError {
    match error {
        TxError::Storage(storage) => ExtensionJobStoreError::Storage(storage),
        TxError::Aborted(reason) => reason,
    }
}
