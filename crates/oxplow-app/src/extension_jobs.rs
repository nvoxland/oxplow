//! Public intent/result services for declared durable jobs. Native command
//! handlers own the transaction; registered operations own external execution.

use crate::{
    extension_catalog::{CatalogSnapshot, ExtensionCatalog},
    extension_job_ai::TypedAiJobRequest,
    extension_job_authority::{DeclaredJobGrants, JobAuthorityHost},
    extension_job_dispatch::ExtensionJobDispatcher,
    extension_job_operations::{
        JobOperation, JobOperationError, JobOperationKey, JobOperationRegistry, JobOperations,
        JobWork, JobWorkOutcome, RegisteredJobOperation,
    },
};
use oxplow_db::{
    extension_job_store::{
        self, EnqueueResult, ExtensionJobStoreError, NewExtensionJob, SqliteExtensionJobStore,
    },
    Database,
};
use oxplow_domain::{
    contribution::{ContributionId, ContributionKind},
    extension_job::{
        Completion, ExtensionJob, IntentKey, JobEvent, JobId, JobOwner, JobState, ReplayPolicy,
    },
    extension_job_context::JobInvocationContext,
    run::{Cancel, RunContext},
    CommandError,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnqueueInput {
    pub job: String,
    pub intent_key: String,
    pub request: Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    pub id: String,
}

pub struct JobsDependencies {
    pub db: Database,
    pub catalog: Arc<ExtensionCatalog>,
    pub root: PathBuf,
    pub ai: Arc<crate::ai_compute::AiCompute>,
    pub approvals: Arc<crate::exec_consent::ApprovalStore>,
    pub threads: Arc<dyn oxplow_domain::stores::ThreadStore>,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub sql: crate::sql_gateway::SqlGateway,
}

pub struct ExtensionJobs {
    db: Database,
    catalog: Arc<ExtensionCatalog>,
    root: PathBuf,
    ownership: crate::extension_state::StateHost,
    operations: JobOperations,
    ai: Arc<crate::ai_compute::AiCompute>,
    approvals: Arc<crate::exec_consent::ApprovalStore>,
    threads: Arc<dyn oxplow_domain::stores::ThreadStore>,
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    sql: crate::sql_gateway::SqlGateway,
    spawned: std::sync::atomic::AtomicBool,
    shutdown: Cancel,
}

impl ExtensionJobs {
    pub fn new(deps: JobsDependencies) -> Self {
        let JobsDependencies {
            db,
            catalog,
            root,
            ai,
            approvals,
            threads,
            vocabulary,
            sql,
        } = deps;
        Self {
            ownership: crate::extension_state::StateHost::new(catalog.clone(), root.clone()),
            db,
            catalog,
            root,
            ai,
            approvals,
            threads,
            vocabulary,
            sql,
            operations: JobOperations::default(),
            spawned: Default::default(),
            shutdown: Cancel::default(),
        }
    }

    pub(crate) fn prepare_registry(
        &self,
        snapshot: &CatalogSnapshot,
    ) -> Result<Arc<JobOperationRegistry>, crate::vocabulary_reactor::VocabularyErrors> {
        let mut operations = vec![];
        for extension in snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
        {
            for job in &extension.jobs {
                let owner = JobOwner {
                    contribution: ContributionId::new(
                        &extension.name,
                        ContributionKind::Job,
                        &job.id,
                    ),
                    package_version: extension.version.clone(),
                    generation: snapshot.generation().into(),
                    contract_version: Some(job.version),
                };
                let key = JobOperationKey::new(owner, job.operation.as_str().into())
                    .map_err(|_| job_problem())?;
                operations.push(Arc::new(
                    RegisteredJobOperation::new(
                        key,
                        replay(job.replay),
                        Arc::new(AiJob {
                            ai: self.ai.clone(),
                        }),
                    )
                    .map_err(|_| job_problem())?,
                ));
            }
        }
        JobOperationRegistry::new(operations)
            .map(Arc::new)
            .map_err(|_| job_problem())
    }

    pub(crate) fn publish_registry(&self, registry: Arc<JobOperationRegistry>) {
        self.operations.publish(registry);
    }

    /// Public author path, called inside an audited native command transaction.
    pub fn enqueue(
        &self,
        ctx: &crate::commands::TxCtx<'_>,
        input: &EnqueueInput,
    ) -> Result<Value, CommandError> {
        if ctx.conn.is_autocommit() {
            return Err(denied("job intent requires an audited transaction"));
        }
        if input.intent_key.len() > 128 || input.job.len() > 256 {
            return Err(invalid("invalid job declaration or intent key"));
        }
        let request = TypedAiJobRequest::from_value(&input.request)
            .map_err(|_| invalid("invalid typed AI job input"))?;
        let intent_key = IntentKey::new(input.intent_key.clone())
            .map_err(|_| invalid("invalid job intent key"))?;
        let (owner, operation, policy) = self
            .ownership
            .with_owner(ctx.run, "jobs.write", |extension| {
                extension
                    .jobs
                    .iter()
                    .find(|job| job.id == input.job)
                    .map(|job| {
                        (
                            JobOwner {
                                contribution: ContributionId::new(
                                    &extension.name,
                                    ContributionKind::Job,
                                    &job.id,
                                ),
                                package_version: extension.version.clone(),
                                generation: ctx
                                    .run
                                    .contribution_ref()
                                    .and_then(|owner| owner.generation.clone())
                                    .expect("validated runtime owner"),
                                contract_version: Some(job.version),
                            },
                            job.operation.as_str().to_owned(),
                            replay(job.replay),
                        )
                    })
            })
            .map_err(|_| denied("job scheduling authority is unavailable"))?
            .ok_or_else(|| invalid("declared job is unavailable"))?;
        self.operations
            .snapshot()
            .find(&owner, &operation)
            .map_err(|_| denied("job operation is unavailable"))?;
        let context = JobInvocationContext::capture(ctx.run)
            .map_err(|_| invalid("invalid job scheduling provenance"))?;
        let job = ExtensionJob::queued(
            JobId::generate(),
            owner,
            ctx.run.run().clone(),
            intent_key,
            policy,
            oxplow_domain::Timestamp::now().unix_ms(),
        )
        .map_err(|_| invalid("invalid job intent"))?;
        let new = NewExtensionJob {
            job,
            operation,
            invoker: ctx.run.actor().source(),
            request: serde_json::to_value(request).expect("validated request serializes"),
        };
        // A duplicate is allowed even at capacity; the immutable input and
        // provenance checks in enqueue prevent reusing an intent for new work.
        let existing: bool = ctx.conn.query_row("SELECT EXISTS(SELECT 1 FROM extension_job WHERE contribution_id=?1 AND intent_key=?2)",
            rusqlite::params![new.job.owner.contribution.to_string(), new.job.intent_key.as_str()], |row| row.get(0))
            .map_err(oxplow_db::map_sql_err)?;
        if !existing {
            let (project, package): (i64, i64) = ctx
                .conn
                .query_row(
                    "SELECT count(*), COALESCE(sum(contribution_id LIKE ?1), 0)
                FROM extension_job WHERE state IN ('queued', 'running')",
                    [format!("{}/job/%", new.job.owner.contribution.extension)],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)?;
            if project >= 1024 || package >= 256 {
                return Err(invalid("job queue capacity reached"));
            }
        }
        let kept = extension_job_store::enqueue_with_context_tx(ctx.conn, &new, &context)
            .map_err(store_error)?;
        let (created, stored) = match kept {
            EnqueueResult::Created(job) => (true, job),
            EnqueueResult::Existing(job) => (false, job),
        };
        Ok(json!({"id": stored.job.id.as_str(), "state": stored.job.state, "created": created}))
    }

    pub fn snapshot(
        &self,
        conn: &rusqlite::Connection,
        run: &RunContext,
        input: &JobInput,
    ) -> Result<Value, CommandError> {
        let id = job_id(&input.id)?;
        let extension = self
            .ownership
            .with_owner(run, "jobs.read", |extension| extension.name.clone())
            .map_err(|_| denied("job result authority is unavailable"))?;
        let metadata = extension_job_store::metadata_tx(conn, &id)
            .map_err(store_error)?
            .filter(|row| row.job.owner.contribution.extension == extension)
            .ok_or_else(|| invalid("job was not found"))?;
        check_stream(
            run,
            &extension_job_store::get_context_tx(conn, &id).map_err(store_error)?,
        )?;
        let result = if metadata.job.attempt > 0 {
            extension_job_store::attempts_tx(conn, &id, Some(metadata.job.attempt - 1), 1)
                .map_err(store_error)?
                .items
                .into_iter()
                .next()
                .and_then(|attempt| attempt.result)
        } else {
            None
        };
        Ok(
            json!({"id": id.as_str(), "job": metadata.job.owner.contribution.to_string(),
            "version": metadata.job.owner.contract_version, "state": metadata.job.state,
            "attempt": metadata.job.attempt, "cancellationRequested": metadata.job.cancellation_requested_at_ms.is_some(), "result": result}),
        )
    }

    /// Worker readers re-enter their retained generation before private DB
    /// dispatch. The run is captured by the DB closure, preserving that pin if
    /// its async caller is cancelled while the worker is still reading.
    pub(crate) async fn snapshot_fresh(
        self: &Arc<Self>,
        run: RunContext,
        input: JobInput,
    ) -> Result<Value, CommandError> {
        let host = self.clone();
        let retained = self.sql.pin_run(&run).await.map_err(CommandError::from)?;
        let reader = retained.clone();
        self.sql
            .read_in(
                &retained,
                self.db.read_or(move |conn| {
                    host.snapshot(conn, &reader, &input)
                        .map_err(oxplow_db::TxError::Aborted)
                }),
            )
            .await
            .map_err(CommandError::from)?
            .map_err(|error| match error {
                oxplow_db::TxError::Aborted(error) => error,
                oxplow_db::TxError::Storage(error) => error.into(),
            })
    }

    pub fn cancel(
        &self,
        ctx: &crate::commands::TxCtx<'_>,
        input: &JobInput,
    ) -> Result<Value, CommandError> {
        if ctx.conn.is_autocommit() {
            return Err(denied("job cancellation requires an audited transaction"));
        }
        let id = job_id(&input.id)?;
        let extension = self
            .ownership
            .with_owner(ctx.run, "jobs.write", |extension| extension.name.clone())
            .map_err(|_| denied("job cancellation authority is unavailable"))?;
        let metadata = extension_job_store::metadata_tx(ctx.conn, &id)
            .map_err(store_error)?
            .filter(|row| row.job.owner.contribution.extension == extension)
            .ok_or_else(|| invalid("job was not found"))?;
        check_stream(
            ctx.run,
            &extension_job_store::get_context_tx(ctx.conn, &id).map_err(store_error)?,
        )?;
        if !matches!(
            metadata.job.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        ) && metadata.job.cancellation_requested_at_ms.is_none()
        {
            extension_job_store::transition_tx(
                ctx.conn,
                &id,
                metadata.revision,
                &JobEvent::RequestCancellation {
                    now_ms: oxplow_domain::Timestamp::now().unix_ms(),
                },
                None,
            )
            .map_err(store_error)?;
        }
        Ok(
            json!({"id": id.as_str(), "cancellationRequested": !matches!(metadata.job.state, JobState::Succeeded | JobState::Failed)}),
        )
    }

    pub(crate) fn spawn(
        self: &Arc<Self>,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
    ) {
        if self.spawned.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let grants = Arc::new(DeclaredJobGrants::new(
            self.catalog.clone(),
            self.root.clone(),
            self.approvals.clone(),
        ));
        let host = Arc::new(
            JobAuthorityHost::new(
                self.catalog.clone(),
                self.root.clone(),
                self.operations.snapshot(),
                grants,
                self.threads.clone(),
            )
            .with_registry_handle(self.operations.clone())
            .with_activation(activation),
        );
        let dispatcher = Arc::new(
            ExtensionJobDispatcher::new(self.db.clone(), host)
                .with_vocabulary(self.vocabulary.clone()),
        );
        let scheduler = crate::extension_job_scheduler::ExtensionJobScheduler::new(
            SqliteExtensionJobStore::new(self.db.clone()),
            dispatcher,
            self.shutdown.clone(),
        );
        tokio::spawn(scheduler.run());
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

impl Drop for ExtensionJobs {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct AiJob {
    ai: Arc<crate::ai_compute::AiCompute>,
}

#[async_trait::async_trait]
impl JobOperation for AiJob {
    async fn execute(&self, work: JobWork) -> Result<JobWorkOutcome, JobOperationError> {
        let request = match TypedAiJobRequest::from_value(work.request()) {
            Ok(request) => request,
            Err(_) => {
                return Ok(JobWorkOutcome::Completed {
                    completion: Completion::Failed,
                    result: json!({"error": "invalid typed AI job input"}),
                })
            }
        };
        let future = self.ai.complete_job(work.run(), request);
        tokio::pin!(future);
        loop {
            tokio::select! {
                result = &mut future => return Ok(match result {
                    Ok(value) => JobWorkOutcome::Completed { completion: Completion::Succeeded,
                        result: serde_json::to_value(value).map_err(|_| JobOperationError::Failed)? },
                    Err(_) => JobWorkOutcome::Completed { completion: Completion::Failed,
                        result: json!({"error": "typed AI computation failed"}) },
                }),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if work.run().check_authority().is_err() {
                        // Dropping HTTP work cannot prove the remote computation
                        // stopped or had no effect; retain uncertain evidence.
                        return Ok(JobWorkOutcome::Uncertain);
                    }
                }
            }
        }
    }
}

fn replay(policy: crate::extensions::jobs::JobReplay) -> ReplayPolicy {
    match policy {
        crate::extensions::jobs::JobReplay::Never => ReplayPolicy::Never,
        crate::extensions::jobs::JobReplay::Idempotent => ReplayPolicy::Idempotent,
    }
}
fn job_problem() -> crate::vocabulary_reactor::VocabularyErrors {
    crate::vocabulary_reactor::VocabularyErrors::from([(
        "<host>".into(),
        vec!["job operation registry candidate is invalid".into()],
    )])
}
fn job_id(id: &str) -> Result<JobId, CommandError> {
    if id.len() > 64 {
        return Err(invalid("invalid job identity"));
    }
    serde_json::from_value(Value::String(id.into())).map_err(|_| invalid("invalid job identity"))
}
fn check_stream(run: &RunContext, context: &JobInvocationContext) -> Result<(), CommandError> {
    if run.actor().confined_to_own_stream()
        && run.anchors().stream_id != context.anchors().stream_id
    {
        return Err(denied("job belongs to another stream"));
    }
    Ok(())
}
fn denied(message: &str) -> CommandError {
    CommandError::Denied {
        reason: message.into(),
    }
}
fn invalid(message: &str) -> CommandError {
    CommandError::Invalid {
        field: None,
        message: message.into(),
    }
}
fn store_error(error: ExtensionJobStoreError) -> CommandError {
    match error {
        ExtensionJobStoreError::Storage(error) => error.into(),
        ExtensionJobStoreError::Conflict | ExtensionJobStoreError::IntentConflict => {
            invalid("job intent or revision conflicts with stored work")
        }
        _ => invalid("invalid or unavailable job state"),
    }
}
