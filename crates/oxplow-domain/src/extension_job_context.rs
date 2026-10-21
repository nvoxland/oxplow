//! Bounded, durable scheduling provenance for an extension job.
//!
//! This is a snapshot of attribution and event context only. It cannot be
//! used to construct a runnable context or recover live authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    commands::Actor,
    events::{Anchors, EventId},
    extension_job::{AttemptFence, JobId},
    ids::{AgentSessionId, EffortId, StreamId, ThreadId},
    run::{RunContext, RunId},
    uri::grammar::Uri,
};

const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_ACTOR_DEPTH: usize = 8;
const MAX_SUBJECTS: usize = 64;
const MAX_SUBJECT_BYTES: usize = 2048;
const MAX_LABEL_BYTES: usize = 256;
const MAX_SQLITE_INT: u64 = i64::MAX as u64;

/// Fixed, payload-free validation/encoding errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JobContextError {
    #[error("invalid extension job context")]
    Invalid,
    #[error("extension job context exceeds its size limit")]
    TooLarge,
    #[error("invalid extension job context JSON")]
    InvalidJson,
}

/// Immutable provenance captured when a job is first scheduled.
///
/// Fields are private so callers cannot bypass validation. `Debug` is
/// intentionally omitted because subjects and attribution are private context.
#[derive(Clone, PartialEq, Serialize)]
pub struct JobInvocationContext {
    schema_version: u8,
    originating_run_id: RunId,
    scheduled_by: Actor,
    anchors: Anchors,
    subject: Vec<String>,
    cause: Option<EventId>,
}

impl JobInvocationContext {
    pub const SCHEMA_VERSION: u8 = 1;

    /// Capture only the durable attribution fields from a live run.
    pub fn capture(run: &RunContext) -> Result<Self, JobContextError> {
        validate_run_parts(run)?;
        count_json(&BorrowedContext {
            schema_version: Self::SCHEMA_VERSION,
            originating_run_id: run.run(),
            scheduled_by: run.actor(),
            anchors: run.anchors(),
            subject: run.subject(),
            cause: run.cause(),
        })?;
        let context = Self {
            schema_version: Self::SCHEMA_VERSION,
            originating_run_id: run.run().clone(),
            scheduled_by: run.actor().clone(),
            anchors: run.anchors().clone(),
            subject: run.subject().to_vec(),
            cause: run.cause().cloned(),
        };
        context.validate()?;
        Ok(context)
    }

    pub const fn schema_version(&self) -> u8 {
        self.schema_version
    }

    pub fn originating_run_id(&self) -> &RunId {
        &self.originating_run_id
    }

    pub fn scheduled_by(&self) -> &Actor {
        &self.scheduled_by
    }

    pub fn anchors(&self) -> &Anchors {
        &self.anchors
    }

    pub fn subject(&self) -> &[String] {
        &self.subject
    }

    pub fn cause(&self) -> Option<&EventId> {
        self.cause.as_ref()
    }

    /// Compare scheduling provenance while allowing a duplicate delivery's
    /// newly-created originating run to differ.
    pub fn same_scheduling_provenance(&self, other: &Self) -> bool {
        self.scheduled_by == other.scheduled_by
            && self.anchors == other.anchors
            && self.subject == other.subject
            && self.cause == other.cause
    }

    pub fn validate(&self) -> Result<(), JobContextError> {
        if self.schema_version != Self::SCHEMA_VERSION
            || !canonical_uuid_v7(self.originating_run_id.as_str())
            || !valid_actor(&self.scheduled_by, 1)?
            || !valid_anchors(&self.anchors)
            || !actor_anchors_match(&self.scheduled_by, &self.anchors)
            || self
                .cause
                .as_ref()
                .is_some_and(|cause| !canonical_uuid_v7(cause.as_str()))
            || self.subject.len() > MAX_SUBJECTS
        {
            return Err(JobContextError::Invalid);
        }
        for value in &self.subject {
            validate_subject(value)?;
        }
        if encoded_len(self)? > MAX_JSON_BYTES {
            return Err(JobContextError::TooLarge);
        }
        Ok(())
    }

    /// Encode in canonical field order after counting the output without
    /// allocating a serialized copy.
    pub fn encode_json(&self) -> Result<String, JobContextError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| JobContextError::InvalidJson)
    }

    /// Decode one bounded JSON document and reject unknown fields at every
    /// context, anchor, and actor level.
    pub fn decode_json(raw: &str) -> Result<Self, JobContextError> {
        if raw.len() > MAX_JSON_BYTES {
            return Err(JobContextError::TooLarge);
        }
        let wire: WireContext =
            serde_json::from_str(raw).map_err(|_| JobContextError::InvalidJson)?;
        if wire.schema_version != Self::SCHEMA_VERSION
            || !canonical_uuid_v7(wire.originating_run_id.as_str())
            || !valid_wire_actor(&wire.scheduled_by, 1, &wire.anchors)?
            || !valid_wire_anchors(&wire.anchors)
            || wire
                .cause
                .as_ref()
                .is_some_and(|cause| !canonical_uuid_v7(cause.as_str()))
            || wire.subject.len() > MAX_SUBJECTS
        {
            return Err(JobContextError::Invalid);
        }
        for value in &wire.subject {
            validate_subject(value)?;
        }
        let context = Self {
            schema_version: wire.schema_version,
            originating_run_id: wire.originating_run_id,
            scheduled_by: wire.scheduled_by.into(),
            anchors: wire.anchors.into(),
            subject: wire.subject,
            cause: wire.cause,
        };
        context.validate()?;
        Ok(context)
    }
}

/// The host's execution token for a single dispatch attempt.
///
/// It is deliberately not an Actor and carries no authority semantics.
#[derive(Clone, PartialEq, Eq)]
pub struct JobExecutionIdentity {
    job_id: JobId,
    token: AttemptFence,
}

impl JobExecutionIdentity {
    pub fn new(job_id: JobId, token: AttemptFence) -> Result<Self, JobContextError> {
        let attempt = u64::from(token.attempt);
        let expected_fence = attempt.checked_mul(2).and_then(|n| n.checked_sub(1));
        if !canonical_uuid_v7(job_id.as_str())
            || attempt == 0
            || attempt > MAX_SQLITE_INT
            || token.fence == 0
            || token.fence > MAX_SQLITE_INT
            || expected_fence != Some(token.fence)
        {
            return Err(JobContextError::Invalid);
        }
        Ok(Self { job_id, token })
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub const fn attempt_fence(&self) -> AttemptFence {
        self.token
    }

    /// The run's ref (`job_run:<id>`): what its actor's events and audit
    /// rows name as their source.
    pub fn source(&self) -> String {
        crate::uri::build::job_run_uri(self.job_id.as_str())
    }

    pub const fn may_confirm(&self) -> bool {
        false
    }
}

#[derive(Serialize)]
struct BorrowedContext<'a> {
    schema_version: u8,
    originating_run_id: &'a RunId,
    scheduled_by: &'a Actor,
    anchors: &'a Anchors,
    subject: &'a [String],
    cause: Option<&'a EventId>,
}

struct CountWriter {
    len: usize,
    exceeded: bool,
}

impl std::io::Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let Some(len) = self.len.checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(std::io::Error::other("context size limit"));
        };
        if len > MAX_JSON_BYTES {
            self.exceeded = true;
            return Err(std::io::Error::other("context size limit"));
        }
        self.len = len;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_len(context: &JobInvocationContext) -> Result<usize, JobContextError> {
    count_json(context)
}

fn count_json(value: &impl Serialize) -> Result<usize, JobContextError> {
    let mut writer = CountWriter {
        len: 0,
        exceeded: false,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return if writer.exceeded {
            Err(JobContextError::TooLarge)
        } else {
            Err(JobContextError::InvalidJson)
        };
    }
    Ok(writer.len)
}

fn validate_run_parts(run: &RunContext) -> Result<(), JobContextError> {
    if !canonical_uuid_v7(run.run().as_str())
        || !valid_actor(run.actor(), 1)?
        || !valid_anchors(run.anchors())
        || !actor_anchors_match(run.actor(), run.anchors())
        || run
            .cause()
            .is_some_and(|cause| !canonical_uuid_v7(cause.as_str()))
        || run.subject().len() > MAX_SUBJECTS
    {
        return Err(JobContextError::Invalid);
    }
    for value in run.subject() {
        validate_subject(value)?;
    }
    Ok(())
}

fn validate_subject(value: &str) -> Result<(), JobContextError> {
    if value.is_empty() || value.len() > MAX_SUBJECT_BYTES || value.chars().any(char::is_control) {
        return Err(JobContextError::Invalid);
    }
    let parsed = Uri::parse(value).map_err(|_| JobContextError::Invalid)?;
    if parsed.to_string() != value {
        return Err(JobContextError::Invalid);
    }
    Ok(())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_LABEL_BYTES && !value.chars().any(char::is_control)
}

fn valid_actor(actor: &Actor, depth: usize) -> Result<bool, JobContextError> {
    if depth > MAX_ACTOR_DEPTH {
        return Ok(false);
    }
    Ok(match actor {
        Actor::Human | Actor::System => true,
        Actor::Agent { .. } => true,
        Actor::Effect { effect, .. } => valid_label(effect),
        Actor::Job {
            job_id,
            attempt,
            fence,
            ..
        } => valid_job_actor(job_id, *attempt, *fence),
        Actor::Lens {
            lens_id,
            on_behalf_of,
        } => valid_label(lens_id) && valid_actor(on_behalf_of, depth + 1)?,
        Actor::Component {
            component,
            on_behalf_of,
        } => valid_label(component) && valid_actor(on_behalf_of, depth + 1)?,
    })
}

fn valid_anchors(anchors: &Anchors) -> bool {
    anchors.stream_id.is_none_or(|id| id.value() > 0)
        && anchors.thread_id.is_none_or(|id| id.value() > 0)
        && anchors.effort_id.is_none_or(|id| id.value() > 0)
        && anchors.agent_session_id.is_none_or(|id| id.value() > 0)
        && anchors.turn_id.is_none_or(|id| id > 0)
        && anchors.snapshot_id.is_none_or(|id| id > 0)
}

fn actor_anchors_match(actor: &Actor, anchors: &Anchors) -> bool {
    match actor {
        Actor::Human | Actor::System => true,
        Actor::Agent {
            session_id,
            thread_id,
            stream_id,
        } => {
            actor_id_matches(*stream_id, anchors.stream_id)
                && actor_id_matches(*thread_id, anchors.thread_id)
                && actor_id_matches(*session_id, anchors.agent_session_id)
        }
        Actor::Effect {
            thread_id,
            stream_id,
            ..
        }
        | Actor::Job {
            thread_id,
            stream_id,
            ..
        } => {
            actor_id_matches(*stream_id, anchors.stream_id)
                && actor_id_matches(*thread_id, anchors.thread_id)
        }
        Actor::Lens { on_behalf_of, .. } | Actor::Component { on_behalf_of, .. } => {
            actor_anchors_match(on_behalf_of, anchors)
        }
    }
}

fn actor_id_matches<T: PartialEq>(actor: Option<T>, anchor: Option<T>) -> bool {
    actor.is_none() || actor == anchor
}

fn valid_job_actor(job_id: &str, attempt: u32, fence: u64) -> bool {
    canonical_uuid_v7(job_id)
        && attempt > 0
        && fence == u64::from(attempt) * 2 - 1
        && fence <= i64::MAX as u64
}

fn canonical_uuid_v7(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 7
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == value
    })
}

// Strict wire mirrors are necessary because shared Actor/Anchors serde types
// intentionally tolerate unknown fields. Conversion happens only after the
// whole bounded raw document has passed this strict shape check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContext {
    schema_version: u8,
    originating_run_id: RunId,
    scheduled_by: WireActor,
    anchors: WireAnchors,
    subject: Vec<String>,
    cause: Option<EventId>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireActor {
    // Empty struct variants enforce strict fields, including these two cases.
    Human {},
    Agent {
        session_id: Option<AgentSessionId>,
        thread_id: Option<ThreadId>,
        stream_id: Option<StreamId>,
    },
    Lens {
        lens_id: String,
        on_behalf_of: Box<WireActor>,
    },
    Component {
        component: String,
        on_behalf_of: Box<WireActor>,
    },
    System {},
    Effect {
        effect: String,
        thread_id: Option<ThreadId>,
        stream_id: Option<StreamId>,
    },
    Job {
        job_id: String,
        attempt: u32,
        fence: u64,
        thread_id: Option<ThreadId>,
        stream_id: Option<StreamId>,
    },
}

impl From<WireActor> for Actor {
    fn from(actor: WireActor) -> Self {
        match actor {
            WireActor::Human {} => Self::Human,
            WireActor::Agent {
                session_id,
                thread_id,
                stream_id,
                ..
            } => Self::Agent {
                session_id,
                thread_id,
                stream_id,
            },
            WireActor::Lens {
                lens_id,
                on_behalf_of,
                ..
            } => Self::Lens {
                lens_id,
                on_behalf_of: Box::new((*on_behalf_of).into()),
            },
            WireActor::Component {
                component,
                on_behalf_of,
            } => Self::Component {
                component,
                on_behalf_of: Box::new((*on_behalf_of).into()),
            },
            WireActor::System {} => Self::System,
            WireActor::Job {
                job_id,
                attempt,
                fence,
                thread_id,
                stream_id,
            } => Self::Job {
                job_id,
                attempt,
                fence,
                thread_id,
                stream_id,
            },
            WireActor::Effect {
                effect,
                thread_id,
                stream_id,
                ..
            } => Self::Effect {
                effect,
                thread_id,
                stream_id,
            },
        }
    }
}

fn valid_wire_actor(
    actor: &WireActor,
    depth: usize,
    anchors: &WireAnchors,
) -> Result<bool, JobContextError> {
    if depth > MAX_ACTOR_DEPTH {
        return Ok(false);
    }
    Ok(match actor {
        WireActor::Human {} | WireActor::System {} => true,
        WireActor::Job {
            job_id,
            attempt,
            fence,
            thread_id,
            stream_id,
        } => {
            valid_job_actor(job_id, *attempt, *fence)
                && actor_id_matches(*stream_id, anchors.stream_id)
                && actor_id_matches(*thread_id, anchors.thread_id)
        }
        WireActor::Agent {
            session_id,
            thread_id,
            stream_id,
            ..
        } => {
            actor_id_matches(*stream_id, anchors.stream_id)
                && actor_id_matches(*thread_id, anchors.thread_id)
                && actor_id_matches(*session_id, anchors.agent_session_id)
        }
        WireActor::Lens {
            lens_id,
            on_behalf_of,
            ..
        } => valid_label(lens_id) && valid_wire_actor(on_behalf_of, depth + 1, anchors)?,
        WireActor::Component {
            component,
            on_behalf_of,
        } => valid_label(component) && valid_wire_actor(on_behalf_of, depth + 1, anchors)?,
        WireActor::Effect {
            effect,
            thread_id,
            stream_id,
            ..
        } => {
            valid_label(effect)
                && actor_id_matches(*stream_id, anchors.stream_id)
                && actor_id_matches(*thread_id, anchors.thread_id)
        }
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAnchors {
    stream_id: Option<StreamId>,
    thread_id: Option<ThreadId>,
    effort_id: Option<EffortId>,
    turn_id: Option<i64>,
    agent_session_id: Option<AgentSessionId>,
    snapshot_id: Option<i64>,
}

impl From<WireAnchors> for Anchors {
    fn from(a: WireAnchors) -> Self {
        Self {
            stream_id: a.stream_id,
            thread_id: a.thread_id,
            effort_id: a.effort_id,
            turn_id: a.turn_id,
            agent_session_id: a.agent_session_id,
            snapshot_id: a.snapshot_id,
        }
    }
}

fn valid_wire_anchors(a: &WireAnchors) -> bool {
    a.stream_id.is_none_or(|id| id.value() > 0)
        && a.thread_id.is_none_or(|id| id.value() > 0)
        && a.effort_id.is_none_or(|id| id.value() > 0)
        && a.agent_session_id.is_none_or(|id| id.value() > 0)
        && a.turn_id.is_none_or(|id| id > 0)
        && a.snapshot_id.is_none_or(|id| id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{Mode, RunContext};
    use crate::{StreamId, ThreadId};

    /// A job a custom component scheduled for an agent keeps who scheduled
    /// it, through the strict wire shape and back.
    #[test]
    fn a_job_a_component_scheduled_round_trips() {
        let run = RunContext::core(
            Actor::Component {
                component: "acme/board".into(),
                on_behalf_of: Box::new(Actor::Agent {
                    session_id: None,
                    thread_id: Some(ThreadId::new(3)),
                    stream_id: Some(StreamId::new(1)),
                }),
            },
            Mode::External,
        );
        let context = JobInvocationContext::capture(&run).unwrap();
        let decoded = JobInvocationContext::decode_json(&context.encode_json().unwrap()).unwrap();
        assert_eq!(decoded.scheduled_by(), run.actor());
    }
}
