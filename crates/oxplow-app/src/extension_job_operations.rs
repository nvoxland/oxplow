//! Trusted operation bindings for host-dispatched extension jobs.
//!
//! The registry pins handlers to exact owner metadata. It does not grant
//! authority; dispatch must resolve current ownership and live permission.

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::{
    commands::Actor,
    extension_job::{AttemptFence, Completion, JobId, JobOwner, ReplayPolicy},
    extension_job_context::{JobExecutionIdentity, JobInvocationContext},
    run::{Mode, RunContext},
};
use serde_json::Value;

const MAX_OWNER_FIELD_BYTES: usize = 256;
const MAX_OPERATION_BYTES: usize = 256;
const MAX_REGISTRATIONS: usize = 1024;
const MAX_REQUEST_BYTES: usize = 1_048_576;
const MAX_REQUEST_DEPTH: usize = 64;

/// Fixed operation-registry and request errors. Handler details and payloads
/// are deliberately not included in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JobOperationError {
    #[error("invalid job operation data")]
    InvalidData,
    #[error("duplicate job operation registration")]
    Duplicate,
    #[error("job operation is unavailable")]
    Unavailable,
    #[error("job operation failed")]
    Failed,
}

/// Exact owner metadata plus one validated operation name.
#[derive(Clone, PartialEq, Eq)]
pub struct JobOperationKey {
    owner: JobOwner,
    operation: String,
}

impl JobOperationKey {
    pub fn new(owner: JobOwner, operation: String) -> Result<Self, JobOperationError> {
        validate_owner(&owner)?;
        validate_operation(&operation)?;
        Ok(Self { owner, operation })
    }

    pub fn owner(&self) -> &JobOwner {
        &self.owner
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }
}

/// One invocation with attempt identity and an externally-modeled run,
/// and its bounded request. Construction takes ownership of the JSON value.
pub struct JobWork {
    identity: JobExecutionIdentity,
    run: RunContext,
    provenance: JobInvocationContext,
    operation: String,
    request: Value,
    idempotency_key: String,
}

impl JobWork {
    pub fn new(
        identity: JobExecutionIdentity,
        key: &JobOperationKey,
        run: RunContext,
        request: Value,
        provenance: JobInvocationContext,
    ) -> Result<Self, JobOperationError> {
        validate_run_owner(&run, &key.owner)?;
        validate_work_provenance(&run, &identity, &provenance)?;
        validate_request(&request)?;
        let idempotency_key = format!(
            "job:{}:{}",
            identity.job_id().as_str(),
            key.operation.as_str()
        );
        Ok(Self {
            identity,
            run,
            provenance,
            operation: key.operation.clone(),
            request,
            idempotency_key,
        })
    }

    pub fn identity(&self) -> &JobExecutionIdentity {
        &self.identity
    }

    pub fn job_id(&self) -> &JobId {
        self.identity.job_id()
    }

    pub const fn attempt_fence(&self) -> AttemptFence {
        self.identity.attempt_fence()
    }

    pub fn run(&self) -> &RunContext {
        &self.run
    }

    pub fn provenance(&self) -> &JobInvocationContext {
        &self.provenance
    }

    pub fn request(&self) -> &Value {
        &self.request
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
}

/// A handler's typed record of what happened. `Stopped` is explicit trusted
/// handler evidence that the worker actually stopped; timeouts stay uncertain.
pub enum JobWorkOutcome {
    Completed {
        completion: Completion,
        result: Value,
    },
    Stopped {
        result: Option<Value>,
    },
    Uncertain,
}

#[async_trait]
pub trait JobOperation: Send + Sync {
    async fn execute(&self, work: JobWork) -> Result<JobWorkOutcome, JobOperationError>;
}

/// A trusted host registration. Its key and replay policy are supplied by
/// host catalog validation, never by a job request.
pub struct RegisteredJobOperation {
    key: JobOperationKey,
    replay_policy: ReplayPolicy,
    handler: Arc<dyn JobOperation>,
}

impl RegisteredJobOperation {
    pub fn new(
        key: JobOperationKey,
        replay_policy: ReplayPolicy,
        handler: Arc<dyn JobOperation>,
    ) -> Result<Self, JobOperationError> {
        validate_owner(&key.owner)?;
        validate_operation(&key.operation)?;
        Ok(Self {
            key,
            replay_policy,
            handler,
        })
    }

    pub fn key(&self) -> &JobOperationKey {
        &self.key
    }

    pub const fn replay_policy(&self) -> ReplayPolicy {
        self.replay_policy
    }

    pub async fn execute(&self, work: JobWork) -> Result<JobWorkOutcome, JobOperationError> {
        if work.operation != self.key.operation
            || validate_run_owner(&work.run, &self.key.owner).is_err()
            || validate_work_provenance(&work.run, &work.identity, &work.provenance).is_err()
        {
            return Err(JobOperationError::InvalidData);
        }
        self.handler.execute(work).await
    }
}

/// Immutable, exact-key registry. Returning an `Arc` pins the selected
/// registration even if a caller later builds a replacement registry.
pub struct JobOperationRegistry {
    operations: Vec<Arc<RegisteredJobOperation>>,
}

/// The host's published registry handle. Generation readers retain selected
/// immutable registrations; only the activation owner replaces this map.
#[derive(Clone)]
pub struct JobOperations {
    registry: Arc<parking_lot::RwLock<Arc<JobOperationRegistry>>>,
}

impl JobOperations {
    pub fn new(registry: Arc<JobOperationRegistry>) -> Self {
        Self {
            registry: Arc::new(parking_lot::RwLock::new(registry)),
        }
    }

    pub fn snapshot(&self) -> Arc<JobOperationRegistry> {
        self.registry.read().clone()
    }

    pub(crate) fn publish(&self, registry: Arc<JobOperationRegistry>) {
        *self.registry.write() = registry;
    }
}

impl Default for JobOperations {
    fn default() -> Self {
        Self::new(Arc::new(
            JobOperationRegistry::new(vec![]).expect("empty operation registry"),
        ))
    }
}

impl JobOperationRegistry {
    pub fn new(operations: Vec<Arc<RegisteredJobOperation>>) -> Result<Self, JobOperationError> {
        if operations.len() > MAX_REGISTRATIONS {
            return Err(JobOperationError::InvalidData);
        }
        for (index, operation) in operations.iter().enumerate() {
            validate_owner(&operation.key.owner)?;
            validate_operation(&operation.key.operation)?;
            if operations[..index]
                .iter()
                .any(|previous| previous.key == operation.key)
            {
                return Err(JobOperationError::Duplicate);
            }
        }
        Ok(Self { operations })
    }

    pub fn find(
        &self,
        owner: &JobOwner,
        operation: &str,
    ) -> Result<Arc<RegisteredJobOperation>, JobOperationError> {
        self.operations
            .iter()
            .find(|registered| {
                registered.key.owner == *owner && registered.key.operation == operation
            })
            .cloned()
            .ok_or(JobOperationError::Unavailable)
    }
}

fn validate_owner(owner: &JobOwner) -> Result<(), JobOperationError> {
    let id = &owner.contribution;
    if id.extension.len() > 128
        || id.local.len() > MAX_OWNER_FIELD_BYTES
        || id.problem().is_some()
        || owner
            .package_version
            .as_deref()
            .is_some_and(|value| !valid_graphic(value, MAX_OWNER_FIELD_BYTES))
        || !valid_graphic(&owner.generation, MAX_OWNER_FIELD_BYTES)
        || owner.contract_version == Some(0)
    {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

fn validate_operation(operation: &str) -> Result<(), JobOperationError> {
    if !valid_graphic(operation, MAX_OPERATION_BYTES) {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

fn valid_graphic(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn validate_run_owner(run: &RunContext, owner: &JobOwner) -> Result<(), JobOperationError> {
    if run.mode() != Mode::External
        || run.cancel().is_cancelled()
        || !run.contribution_ref().is_some_and(|reference| {
            reference.id == owner.contribution
                && reference.version.as_deref() == owner.package_version.as_deref()
                && reference.generation.as_deref() == Some(owner.generation.as_str())
        })
    {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

fn validate_work_provenance(
    run: &RunContext,
    identity: &JobExecutionIdentity,
    provenance: &JobInvocationContext,
) -> Result<(), JobOperationError> {
    provenance
        .validate()
        .map_err(|_| JobOperationError::InvalidData)?;
    let token = identity.attempt_fence();
    let actor_matches = matches!(
        run.actor(),
        Actor::Job {
            job_id,
            attempt,
            fence,
            thread_id,
            stream_id,
        } if job_id == identity.job_id().as_str()
            && *attempt == token.attempt
            && *fence == token.fence
            && *thread_id == provenance.anchors().thread_id
            && *stream_id == provenance.anchors().stream_id
    );
    if !actor_matches
        || run.parent() != Some(provenance.originating_run_id())
        || run.anchors() != provenance.anchors()
        || run.subject() != provenance.subject()
        || run.cause() != provenance.cause()
    {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

fn validate_request(request: &Value) -> Result<(), JobOperationError> {
    let mut lower_bound = 0usize;
    request_lower_bound(request, 1, &mut lower_bound)?;
    let mut writer = RequestCountWriter::default();
    if serde_json::to_writer(&mut writer, request).is_err() {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

fn request_lower_bound(
    value: &Value,
    depth: usize,
    bytes: &mut usize,
) -> Result<(), JobOperationError> {
    if depth > MAX_REQUEST_DEPTH {
        return Err(JobOperationError::InvalidData);
    }
    match value {
        Value::Null => add_request_bytes(bytes, 4),
        Value::Bool(true) => add_request_bytes(bytes, 4),
        Value::Bool(false) => add_request_bytes(bytes, 5),
        Value::Number(_) => add_request_bytes(bytes, 1),
        Value::String(value) => add_request_bytes(bytes, value.len().saturating_add(2)),
        Value::Array(values) => {
            add_request_bytes(bytes, 2)?;
            for (index, value) in values.iter().enumerate() {
                add_request_bytes(bytes, usize::from(u8::from(index > 0)))?;
                request_lower_bound(value, depth + 1, bytes)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            add_request_bytes(bytes, 2)?;
            for (index, (key, value)) in values.iter().enumerate() {
                add_request_bytes(bytes, usize::from(u8::from(index > 0)))?;
                add_request_bytes(bytes, key.len().saturating_add(3))?;
                request_lower_bound(value, depth + 1, bytes)?;
            }
            Ok(())
        }
    }
}

fn add_request_bytes(bytes: &mut usize, amount: usize) -> Result<(), JobOperationError> {
    *bytes = bytes.saturating_add(amount);
    if *bytes > MAX_REQUEST_BYTES {
        return Err(JobOperationError::InvalidData);
    }
    Ok(())
}

#[derive(Default)]
struct RequestCountWriter {
    bytes: usize,
}

impl std::io::Write for RequestCountWriter {
    fn write(&mut self, chunk: &[u8]) -> std::io::Result<usize> {
        let Some(bytes) = self.bytes.checked_add(chunk.len()) else {
            return Err(std::io::Error::other("size limit"));
        };
        if bytes > MAX_REQUEST_BYTES {
            return Err(std::io::Error::other("size limit"));
        }
        self.bytes = bytes;
        Ok(chunk.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
