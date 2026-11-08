//! Pure lifecycle transitions for host-managed durable extension jobs.
//!
//! Job identity and authority are host-owned. This module stores no payload,
//! resolves no handler, performs no IO, and makes no exactly-once guarantee.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{contribution::ContributionId, run::RunId};

const MAX_PERSISTED_COUNTER: u64 = i64::MAX as u64;

/// How many attempts an `Idempotent` job gets in all: one cut off is run
/// again until this many have started, and the last one's end is final.
pub const MAX_ATTEMPTS: u32 = 3;
const MAX_INTENT_KEY_BYTES: usize = 128;
const MAX_OWNER_IDENTITY_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobId(String);

impl JobId {
    /// Generate the host-owned UUIDv7 job identity.
    pub fn generate() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), TransitionError> {
        if is_uuid_v7(&self.0) {
            Ok(())
        } else {
            Err(TransitionError::InvalidIdentity)
        }
    }
}

/// Stable opaque deduplication identity selected by the host, independent of
/// mutable arguments. It is retained unchanged across an allowed retry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentKey {
    value: String,
}

impl IntentKey {
    pub fn new(value: String) -> Result<Self, TransitionError> {
        let key = Self { value };
        key.validate()?;
        Ok(key)
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    fn validate(&self) -> Result<(), TransitionError> {
        if self.value.is_empty()
            || self.value.len() > MAX_INTENT_KEY_BYTES
            || !self.value.is_ascii()
            || !self.value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(TransitionError::InvalidIntentKey);
        }
        Ok(())
    }
}

/// Immutable attribution supplied from the host's captured RunContext and
/// catalog snapshot, never from job arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobOwner {
    /// The job, named in its namespace: whichever extension declares it
    /// there now runs it, so a job outlives a move between siblings.
    pub contribution: ContributionId,
    /// Exact optional package identity from `ContributionRef::version`.
    /// A private unversioned extension does not get an invented package version.
    pub package_version: Option<String>,
    /// Optional public operation contract version, independent of package
    /// version and engine compatibility.
    pub contract_version: Option<u32>,
    /// The hash of the deployment whose declaration owns the job. A
    /// redeploy is another deployment whatever its version says; a queued
    /// job is rebound to it ([`JobEvent::Rebind`]), a running one stops.
    #[serde(default)]
    pub deployment: Option<String>,
}

impl JobOwner {
    fn validate(&self) -> Result<(), TransitionError> {
        if self.contribution.namespace.len() > 128
            || self.contribution.local.len() > MAX_OWNER_IDENTITY_BYTES
            || self.contribution.problem().is_some()
            || self
                .package_version
                .as_deref()
                .is_some_and(|version| !valid_owner_identity(version))
            || self.contract_version == Some(0)
            || self
                .deployment
                .as_deref()
                .is_some_and(|hash| !valid_owner_identity(hash))
        {
            return Err(TransitionError::InvalidOwner);
        }
        Ok(())
    }
}

/// Replay guarantees are selected by the host from a validated operation
/// declaration, never accepted from a caller's request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayPolicy {
    Never,
    Idempotent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Interrupted,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    Succeeded,
    Failed,
}

/// What one row of a running job's output is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutputKind {
    /// Streamed reply text: `{ text }`, a coalesced run of deltas.
    Text,
    /// A tool the model called: `{ id, name, arguments }`.
    ToolCall,
    /// What the tool answered: `{ callId, result }` or `{ callId, error }`.
    ToolResult,
    /// A model round's usage: `{ inputTokens, outputTokens, source, aiCallId }`.
    Usage,
}

impl JobOutputKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            JobOutputKind::Text => "text",
            JobOutputKind::ToolCall => "tool_call",
            JobOutputKind::ToolResult => "tool_result",
            JobOutputKind::Usage => "usage",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "text" => JobOutputKind::Text,
            "tool_call" => JobOutputKind::ToolCall,
            "tool_result" => JobOutputKind::ToolResult,
            "usage" => JobOutputKind::Usage,
            _ => return None,
        })
    }
}

/// One row of output, as written: its sequence number is the store's.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJobOutput {
    pub kind: JobOutputKind,
    pub at_ms: i64,
    pub content: serde_json::Value,
}

/// One row of output, as read.
#[derive(Debug, Clone, PartialEq)]
pub struct JobOutput {
    pub seq: u64,
    pub kind: JobOutputKind,
    pub at_ms: i64,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFence {
    pub attempt: u32,
    pub fence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionJob {
    pub id: JobId,
    pub owner: JobOwner,
    /// Host-generated identity of the originating run; immutable attribution.
    pub run_id: RunId,
    pub intent_key: IntentKey,
    pub replay_policy: ReplayPolicy,
    pub state: JobState,
    /// Zero until the first claim; increases for every dispatch attempt.
    pub attempt: u32,
    /// Monotonic SQLite-safe fencing counter; zero before the first claim.
    pub fence: u64,
    /// Lease expiry in Unix milliseconds, present only while running.
    pub lease_until_ms: Option<i64>,
    /// First durable cancellation request, never cleared or rewritten.
    pub cancellation_requested_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum JobEvent {
    Claim {
        now_ms: i64,
        lease_ms: i64,
        intent_key: IntentKey,
    },
    Renew {
        now_ms: i64,
        lease_ms: i64,
        token: AttemptFence,
    },
    Complete {
        now_ms: i64,
        token: AttemptFence,
        outcome: Completion,
    },
    RequestCancellation {
        now_ms: i64,
    },
    /// The host observed the worker/process/provider stop while the lease and
    /// fence were still current. A request alone is never enough.
    ConfirmStopped {
        now_ms: i64,
        token: AttemptFence,
    },
    /// Recovery observes expiry; this invalidates the old token but does not
    /// claim that the worker stopped or that its external effect did not land.
    RecoverExpired {
        now_ms: i64,
    },
    RetryInterrupted {
        now_ms: i64,
        intent_key: IntentKey,
    },
    /// Its extension was redeployed before this attempt ran: the new
    /// deployment of the same contribution and contract owns it.
    Rebind {
        now_ms: i64,
        owner: JobOwner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTransition {
    pub job: ExtensionJob,
    /// Returned only by Claim; required for every attempt-bound event.
    pub token: Option<AttemptFence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("invalid job identity")]
    InvalidIdentity,
    #[error("invalid job owner")]
    InvalidOwner,
    #[error("invalid intent key")]
    InvalidIntentKey,
    #[error("invalid job state")]
    InvalidState,
    #[error("invalid timestamp or lease")]
    InvalidTime,
    #[error("transition time moved backwards")]
    TimeWentBackwards,
    #[error("attempt fence does not match")]
    FenceMismatch,
    #[error("attempt lease has expired")]
    LeaseExpired,
    #[error("attempt lease has not expired")]
    LeaseActive,
    #[error("intent key does not match the job")]
    IntentKeyMismatch,
    #[error("job transition is not allowed")]
    InvalidTransition,
    #[error("job replay policy does not allow automatic retry")]
    ReplayNotAllowed,
    #[error("job counter limit reached")]
    CounterOverflow,
}

impl ExtensionJob {
    /// Build a queued job. IDs, attribution, intent key and replay policy are
    /// all supplied by the host; arguments are deliberately not represented.
    pub fn queued(
        id: JobId,
        owner: JobOwner,
        run_id: RunId,
        intent_key: IntentKey,
        replay_policy: ReplayPolicy,
        now_ms: i64,
    ) -> Result<Self, TransitionError> {
        if now_ms < 0 {
            return Err(TransitionError::InvalidTime);
        }
        let job = Self {
            id,
            owner,
            run_id,
            intent_key,
            replay_policy,
            state: JobState::Queued,
            attempt: 0,
            fence: 0,
            lease_until_ms: None,
            cancellation_requested_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        job.validate()?;
        Ok(job)
    }

    /// Validate data after deserialization and before it enters a transition.
    pub fn validate(&self) -> Result<(), TransitionError> {
        self.id.validate()?;
        self.owner.validate()?;
        self.intent_key.validate()?;
        if !is_uuid_v7(self.run_id.as_str())
            || self.created_at_ms < 0
            || self.updated_at_ms < self.created_at_ms
            || self.fence > MAX_PERSISTED_COUNTER
            || self.attempt as u64 > self.fence
            || (self.replay_policy == ReplayPolicy::Never && self.attempt > 1)
            || self.attempt > MAX_ATTEMPTS
        {
            return Err(TransitionError::InvalidState);
        }
        if let Some(requested_at) = self.cancellation_requested_at_ms {
            if requested_at < self.created_at_ms || requested_at > self.updated_at_ms {
                return Err(TransitionError::InvalidState);
            }
        }
        // These are the reachable counters of this lifecycle: every claim
        // advances once and each interrupted attempt advances once more.
        let recovered_fence = u64::from(self.attempt) * 2;
        let claimed_fence = recovered_fence.saturating_sub(1);
        match self.state {
            JobState::Queued => {
                if self.lease_until_ms.is_some()
                    || self.cancellation_requested_at_ms.is_some()
                    || ((self.attempt == 0) != (self.fence == 0))
                    || self.fence != recovered_fence
                    || (self.attempt > 0 && self.replay_policy != ReplayPolicy::Idempotent)
                {
                    return Err(TransitionError::InvalidState);
                }
            }
            JobState::Running => {
                if self.attempt == 0
                    || self.fence == 0
                    || self.fence != claimed_fence
                    || self
                        .lease_until_ms
                        .is_none_or(|until| until <= self.created_at_ms)
                {
                    return Err(TransitionError::InvalidState);
                }
            }
            JobState::Interrupted => {
                if self.attempt == 0
                    || self.fence != recovered_fence
                    || self.lease_until_ms.is_some()
                {
                    return Err(TransitionError::InvalidState);
                }
            }
            JobState::Succeeded | JobState::Failed => {
                if self.attempt == 0 || self.fence != claimed_fence || self.lease_until_ms.is_some()
                {
                    return Err(TransitionError::InvalidState);
                }
            }
            JobState::Cancelled => {
                if self.lease_until_ms.is_some()
                    || self.cancellation_requested_at_ms.is_none()
                    || ((self.attempt == 0) != (self.fence == 0))
                    || (self.attempt > 0 && self.fence != claimed_fence)
                {
                    return Err(TransitionError::InvalidState);
                }
            }
        }
        Ok(())
    }

    /// Apply one pure lifecycle event. The original value is never mutated;
    /// invalid transitions return an error without changing it.
    pub fn transition(&self, event: JobEvent) -> Result<JobTransition, TransitionError> {
        self.validate()?;
        let mut next = self.clone();
        let mut issued_token = None;

        match event {
            JobEvent::Claim {
                now_ms,
                lease_ms,
                intent_key,
            } => {
                next.check_time(now_ms)?;
                next.check_intent(&intent_key)?;
                if next.state != JobState::Queued || next.cancellation_requested_at_ms.is_some() {
                    return Err(TransitionError::InvalidTransition);
                }
                let expiry = lease_expiry(now_ms, lease_ms)?;
                next.attempt = next
                    .attempt
                    .checked_add(1)
                    .ok_or(TransitionError::CounterOverflow)?;
                next.fence = next
                    .fence
                    .checked_add(1)
                    .filter(|value| *value <= MAX_PERSISTED_COUNTER)
                    .ok_or(TransitionError::CounterOverflow)?;
                let token = AttemptFence {
                    attempt: next.attempt,
                    fence: next.fence,
                };
                next.state = JobState::Running;
                next.lease_until_ms = Some(expiry);
                next.updated_at_ms = now_ms;
                issued_token = Some(token);
            }
            JobEvent::Renew {
                now_ms,
                lease_ms,
                token,
            } => {
                next.check_time(now_ms)?;
                next.check_running_token(token, now_ms)?;
                let expiry = lease_expiry(now_ms, lease_ms)?;
                if expiry < next.lease_until_ms.ok_or(TransitionError::InvalidState)? {
                    return Err(TransitionError::InvalidTime);
                }
                next.lease_until_ms = Some(expiry);
                next.updated_at_ms = now_ms;
            }
            JobEvent::Complete {
                now_ms,
                token,
                outcome,
            } => {
                next.check_time(now_ms)?;
                next.check_running_token(token, now_ms)?;
                next.state = match outcome {
                    Completion::Succeeded => JobState::Succeeded,
                    Completion::Failed => JobState::Failed,
                };
                next.lease_until_ms = None;
                next.updated_at_ms = now_ms;
            }
            JobEvent::RequestCancellation { now_ms } => {
                if matches!(
                    next.state,
                    JobState::Succeeded | JobState::Failed | JobState::Cancelled
                ) {
                    return Err(TransitionError::InvalidTransition);
                }
                next.check_time(now_ms)?;
                if next.cancellation_requested_at_ms.is_none() {
                    next.cancellation_requested_at_ms = Some(now_ms);
                    if next.state == JobState::Queued {
                        // A requeued job may still have an expired attempt's
                        // external worker/effect in flight. Cancelling its next
                        // dispatch does not establish the old attempt stopped.
                        next.state = if next.attempt == 0 {
                            JobState::Cancelled
                        } else {
                            JobState::Interrupted
                        };
                    }
                }
                next.updated_at_ms = now_ms;
            }
            JobEvent::ConfirmStopped { now_ms, token } => {
                next.check_time(now_ms)?;
                next.check_running_token(token, now_ms)?;
                if next.cancellation_requested_at_ms.is_none() {
                    return Err(TransitionError::InvalidTransition);
                }
                next.state = JobState::Cancelled;
                next.lease_until_ms = None;
                next.updated_at_ms = now_ms;
            }
            JobEvent::RecoverExpired { now_ms } => {
                next.check_time(now_ms)?;
                if next.state != JobState::Running {
                    return Err(TransitionError::InvalidTransition);
                }
                let expiry = next.lease_until_ms.ok_or(TransitionError::InvalidState)?;
                if now_ms < expiry {
                    return Err(TransitionError::LeaseActive);
                }
                next.fence = next
                    .fence
                    .checked_add(1)
                    .filter(|value| *value <= MAX_PERSISTED_COUNTER)
                    .ok_or(TransitionError::CounterOverflow)?;
                next.state = JobState::Interrupted;
                next.lease_until_ms = None;
                next.updated_at_ms = now_ms;
            }
            JobEvent::RetryInterrupted { now_ms, intent_key } => {
                next.check_time(now_ms)?;
                next.check_intent(&intent_key)?;
                if next.state != JobState::Interrupted
                    || next.replay_policy != ReplayPolicy::Idempotent
                    || next.cancellation_requested_at_ms.is_some()
                    || next.attempt >= MAX_ATTEMPTS
                {
                    return Err(TransitionError::ReplayNotAllowed);
                }
                next.state = JobState::Queued;
                next.updated_at_ms = now_ms;
            }
            JobEvent::Rebind { now_ms, owner } => {
                next.check_time(now_ms)?;
                owner.validate()?;
                if next.state != JobState::Queued
                    || next.cancellation_requested_at_ms.is_some()
                    || owner.contribution != next.owner.contribution
                    || owner.contract_version != next.owner.contract_version
                {
                    return Err(TransitionError::InvalidTransition);
                }
                next.owner = owner;
                next.updated_at_ms = now_ms;
            }
        }

        next.validate()?;
        Ok(JobTransition {
            job: next,
            token: issued_token,
        })
    }

    fn check_time(&self, now_ms: i64) -> Result<(), TransitionError> {
        if now_ms < 0 {
            return Err(TransitionError::InvalidTime);
        }
        if now_ms < self.updated_at_ms {
            return Err(TransitionError::TimeWentBackwards);
        }
        Ok(())
    }

    fn check_intent(&self, intent_key: &IntentKey) -> Result<(), TransitionError> {
        intent_key.validate()?;
        if intent_key != &self.intent_key {
            return Err(TransitionError::IntentKeyMismatch);
        }
        Ok(())
    }

    fn check_running_token(&self, token: AttemptFence, now_ms: i64) -> Result<(), TransitionError> {
        if self.state != JobState::Running
            || token.attempt != self.attempt
            || token.fence != self.fence
            || token.attempt == 0
            || token.fence == 0
        {
            return Err(TransitionError::FenceMismatch);
        }
        if now_ms >= self.lease_until_ms.ok_or(TransitionError::InvalidState)? {
            return Err(TransitionError::LeaseExpired);
        }
        Ok(())
    }
}

fn lease_expiry(now_ms: i64, lease_ms: i64) -> Result<i64, TransitionError> {
    if now_ms < 0 || lease_ms <= 0 {
        return Err(TransitionError::InvalidTime);
    }
    now_ms
        .checked_add(lease_ms)
        .filter(|expiry| *expiry > now_ms)
        .ok_or(TransitionError::InvalidTime)
}

fn is_uuid_v7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|uuid| {
        uuid.get_version_num() == 7
            && uuid.get_variant() == uuid::Variant::RFC4122
            && uuid.to_string() == value
    })
}

fn valid_owner_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OWNER_IDENTITY_BYTES
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::ContributionKind;

    const LEASE: i64 = 1_000;

    fn key() -> IntentKey {
        IntentKey::new("review:1".into()).unwrap()
    }

    fn queued(replay: ReplayPolicy) -> ExtensionJob {
        ExtensionJob::queued(
            JobId::generate(),
            JobOwner {
                contribution: ContributionId::new("acme", ContributionKind::Job, "review"),
                package_version: None,
                contract_version: Some(1),
                deployment: Some("d1".into()),
            },
            RunId::generate(),
            key(),
            replay,
            0,
        )
        .unwrap()
    }

    /// Claim `job` at `now`, then let its lease run out: the job recovery
    /// finds, `Interrupted`.
    fn claimed_then_expired(job: ExtensionJob, now: i64) -> ExtensionJob {
        let running = job
            .transition(JobEvent::Claim {
                now_ms: now,
                lease_ms: LEASE,
                intent_key: key(),
            })
            .unwrap()
            .job;
        running
            .transition(JobEvent::RecoverExpired {
                now_ms: now + LEASE,
            })
            .unwrap()
            .job
    }

    fn retry(job: &ExtensionJob, now: i64) -> Result<JobTransition, TransitionError> {
        job.transition(JobEvent::RetryInterrupted {
            now_ms: now,
            intent_key: key(),
        })
    }

    /// An idempotent job cut off is queued again, keeping its attempt; its
    /// next claim is the next attempt, under a new fence.
    /// A queued job whose deployment is replaced is rebound to the new
    /// one of the same contribution and contract; nothing else moves, and
    /// a running job, another contribution or another contract can't be.
    #[test]
    fn a_queued_job_is_rebound_to_its_new_deployment() {
        let job = queued(ReplayPolicy::Never);
        let mut owner = job.owner.clone();
        owner.deployment = Some("d2".into());
        owner.package_version = Some("1.0.1".into());
        let next = job
            .transition(JobEvent::Rebind {
                now_ms: 1,
                owner: owner.clone(),
            })
            .unwrap()
            .job;
        assert_eq!(next.owner, owner);
        assert_eq!(
            (next.state, next.attempt, next.fence),
            (JobState::Queued, 0, 0)
        );

        let mut other = owner.clone();
        other.contract_version = Some(2);
        assert_eq!(
            job.transition(JobEvent::Rebind {
                now_ms: 1,
                owner: other
            }),
            Err(TransitionError::InvalidTransition)
        );
        let running = job
            .transition(JobEvent::Claim {
                now_ms: 1,
                lease_ms: LEASE,
                intent_key: key(),
            })
            .unwrap()
            .job;
        assert_eq!(
            running.transition(JobEvent::Rebind { now_ms: 2, owner }),
            Err(TransitionError::InvalidTransition)
        );
    }

    #[test]
    fn an_interrupted_idempotent_job_is_queued_again_for_its_next_attempt() {
        let interrupted = claimed_then_expired(queued(ReplayPolicy::Idempotent), 10);
        assert_eq!(interrupted.state, JobState::Interrupted);
        let requeued = retry(&interrupted, 2_000).unwrap().job;
        assert_eq!(requeued.state, JobState::Queued);
        assert_eq!(requeued.attempt, 1);
        assert_eq!(requeued.fence, interrupted.fence);
        assert_eq!(requeued.lease_until_ms, None);
        let next = requeued
            .transition(JobEvent::Claim {
                now_ms: 2_001,
                lease_ms: LEASE,
                intent_key: key(),
            })
            .unwrap();
        assert_eq!(next.job.attempt, 2);
        assert_eq!(next.job.fence, interrupted.fence + 1);
        assert_eq!(
            next.token,
            Some(AttemptFence {
                attempt: 2,
                fence: interrupted.fence + 1
            })
        );
    }

    /// A `never` job, one someone asked to stop, and one that has had its
    /// `MAX_ATTEMPTS` aren't replayed.
    #[test]
    fn replay_is_refused_for_never_for_a_cancel_and_past_the_last_attempt() {
        let never = claimed_then_expired(queued(ReplayPolicy::Never), 10);
        assert_eq!(
            retry(&never, 2_000).unwrap_err(),
            TransitionError::ReplayNotAllowed
        );

        let asked = claimed_then_expired(queued(ReplayPolicy::Idempotent), 10)
            .transition(JobEvent::RequestCancellation { now_ms: 2_000 })
            .unwrap()
            .job;
        assert_eq!(
            retry(&asked, 2_001).unwrap_err(),
            TransitionError::ReplayNotAllowed
        );

        let mut job = queued(ReplayPolicy::Idempotent);
        let mut now = 10;
        for attempt in 1..=MAX_ATTEMPTS {
            job = claimed_then_expired(job, now);
            assert_eq!(job.attempt, attempt);
            now += 2 * LEASE;
            if attempt < MAX_ATTEMPTS {
                job = retry(&job, now).unwrap().job;
            }
        }
        assert_eq!(
            retry(&job, now).unwrap_err(),
            TransitionError::ReplayNotAllowed
        );
    }

    /// Cancelling a job queued for its retry ends it `Interrupted`, not
    /// `Cancelled`: the attempt before may still have an effect in flight.
    #[test]
    fn cancelling_a_queued_retry_ends_it_interrupted() {
        let interrupted = claimed_then_expired(queued(ReplayPolicy::Idempotent), 10);
        let requeued = retry(&interrupted, 2_000).unwrap().job;
        let ended = requeued
            .transition(JobEvent::RequestCancellation { now_ms: 2_001 })
            .unwrap()
            .job;
        assert_eq!(ended.state, JobState::Interrupted);
    }
}
