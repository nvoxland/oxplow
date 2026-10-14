//! Public intent/result services for declared durable jobs. Native command
//! handlers own the transaction; registered operations own external execution.

use crate::{
    extension_catalog::{CatalogSnapshot, ExtensionCatalog},
    extension_job_ai::TypedAiJobRequest,
    extension_job_authority::{DeclaredJobGrants, JobAuthorityHost},
    extension_job_dispatch::ExtensionJobDispatcher,
    extension_job_operations::{
        JobOperationKey, JobOperationRegistry, JobOperations, RegisteredJobOperation,
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
    extension_job::{ExtensionJob, IntentKey, JobEvent, JobId, JobOwner, JobState, ReplayPolicy},
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

/// A job run, by its ref (`job_run:<id>`).
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    pub job_run: String,
}

/// How long past its declared `timeout` a job's worker may still be
/// running before the dispatcher stops it (its own timeout fails it first).
const RUNTIME_GRACE: Duration = Duration::from_secs(15);

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
    tools: Arc<crate::extension_job_runner::ToolHost>,
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
            tools: Arc::new(crate::extension_job_runner::ToolHost::new(sql.clone())),
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
                    contract_version: Some(job.version),
                };
                let key = JobOperationKey::new(owner, job.operation.as_str().into())
                    .map_err(|_| job_problem())?;
                let runner = crate::extension_job_runner::AiJob {
                    ai: self.ai.clone(),
                    store: SqliteExtensionJobStore::new(self.db.clone()),
                    tools: self.tools.clone(),
                    definitions: crate::extension_job_runner::AiJob::definitions(
                        job,
                        &extension.commands,
                    ),
                    decl: job.clone(),
                };
                operations.push(Arc::new(
                    RegisteredJobOperation::new(key, replay(job.replay), Arc::new(runner))
                        .map_err(|_| job_problem())?
                        .with_max_runtime(
                            Duration::from_secs(u64::from(job.timeout)) + RUNTIME_GRACE,
                        ),
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
        let intent_key = IntentKey::new(input.intent_key.clone())
            .map_err(|_| invalid("invalid job intent key"))?;
        let (owner, operation, policy, profiles) = self
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
                                contract_version: Some(job.version),
                            },
                            job.operation.as_str().to_owned(),
                            replay(job.replay),
                            extension
                                .ai_profiles
                                .iter()
                                .map(|p| p.id.clone())
                                .collect::<Vec<_>>(),
                        )
                    })
            })
            .map_err(|_| denied("job scheduling authority is unavailable"))?
            .ok_or_else(|| invalid("declared job is unavailable"))?;
        let request = qualify_role(&input.request, &owner.contribution.extension, &profiles)?;
        let request = TypedAiJobRequest::from_value(&request)
            .map_err(|_| invalid("invalid typed AI job input"))?;
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
                    // A job's extension is its run's.
                    "SELECT count(*), COALESCE(sum(r.extension = ?1), 0)
                FROM extension_job j LEFT JOIN extension_run r ON r.run_id = j.run_id
                WHERE j.state IN ('queued', 'running')",
                    [&new.job.owner.contribution.extension],
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
        Ok(
            json!({"jobRun": job_run_ref(&stored.job.id), "state": stored.job.state, "created": created}),
        )
    }

    pub fn snapshot(
        &self,
        conn: &rusqlite::Connection,
        run: &RunContext,
        input: &JobInput,
    ) -> Result<Value, CommandError> {
        let id = job_id(&input.job_run)?;
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
        // What the current attempt has streamed so far: its text, and how
        // many tools it called (`v_job_run_output` has every row).
        let output = if metadata.job.attempt > 0 {
            extension_job_store::output_tx(conn, &id, metadata.job.attempt, 0, usize::MAX)
                .map_err(store_error)?
        } else {
            Vec::new()
        };
        let text: String = output
            .iter()
            .filter(|row| row.kind == oxplow_domain::extension_job::JobOutputKind::Text)
            .filter_map(|row| row.content["text"].as_str())
            .collect();
        let tool_calls = output
            .iter()
            .filter(|row| row.kind == oxplow_domain::extension_job::JobOutputKind::ToolCall)
            .count();
        Ok(json!({
            "jobRun": job_run_ref(&id),
            "job": metadata.job.owner.contribution.to_string(),
            "intentKey": metadata.job.intent_key.as_str(),
            "version": metadata.job.owner.contract_version,
            "state": metadata.job.state,
            "attempt": metadata.job.attempt,
            "cancellationRequested": metadata.job.cancellation_requested_at_ms.is_some(),
            "text": text,
            "toolCalls": tool_calls,
            "result": result,
        }))
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
        let id = job_id(&input.job_run)?;
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
        self.request_cancellation_tx(ctx.conn, &metadata)
    }

    /// A person's cancel of any job (`oxplow.job.cancel`): theirs to stop,
    /// whichever extension started it.
    pub fn cancel_any(
        &self,
        ctx: &crate::commands::TxCtx<'_>,
        input: &JobInput,
    ) -> Result<Value, CommandError> {
        if ctx.conn.is_autocommit() {
            return Err(denied("job cancellation requires an audited transaction"));
        }
        let id = job_id(&input.job_run)?;
        let metadata = extension_job_store::metadata_tx(ctx.conn, &id)
            .map_err(store_error)?
            .ok_or_else(|| invalid("job was not found"))?;
        self.request_cancellation_tx(ctx.conn, &metadata)
    }

    /// Ask `job` to stop, unless it already ended or was asked: a queued
    /// one ends now (and says so), a running one stops at its worker's next
    /// look.
    fn request_cancellation_tx(
        &self,
        conn: &rusqlite::Connection,
        job: &extension_job_store::RecoveryCandidate,
    ) -> Result<Value, CommandError> {
        let ended = matches!(
            job.job.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Interrupted
        );
        if !ended && job.job.cancellation_requested_at_ms.is_none() {
            let changed = extension_job_store::transition_tx(
                conn,
                &job.job.id,
                job.revision,
                &JobEvent::RequestCancellation {
                    now_ms: oxplow_domain::Timestamp::now().unix_ms(),
                },
                None,
            )
            .map_err(store_error)?;
            announce_tx(conn, &self.vocabulary.current(), &changed.job)?;
        }
        Ok(json!({ "jobRun": job_run_ref(&job.job.id), "cancellationRequested": !ended }))
    }

    /// The dispatcher a scheduler drives, over the live registry.
    fn dispatcher(
        &self,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
        commands: Arc<crate::commands::CommandBus>,
    ) -> ExtensionJobDispatcher {
        self.tools.bind(commands);
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
        ExtensionJobDispatcher::new(self.db.clone(), host).with_vocabulary(self.vocabulary.clone())
    }

    /// Start the scheduler (once), as boot does after activation; tool
    /// calls run on `commands`.
    pub fn spawn(
        self: &Arc<Self>,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
        commands: Arc<crate::commands::CommandBus>,
    ) {
        if self.spawned.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let dispatcher = Arc::new(self.dispatcher(activation, commands));
        let scheduler = crate::extension_job_scheduler::ExtensionJobScheduler::new(
            SqliteExtensionJobStore::new(self.db.clone()),
            dispatcher,
            self.shutdown.clone(),
        );
        tokio::spawn(scheduler.run());
    }

    /// A dispatcher a test drives by hand (`run_one`, `recover_page`),
    /// with no scheduler running; `crash` is called at each stage of a
    /// dispatch.
    #[cfg(test)]
    pub(crate) fn dispatcher_for_test(
        &self,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
        commands: Arc<crate::commands::CommandBus>,
        crash: Option<crate::extension_job_dispatch::CrashHook>,
    ) -> Arc<ExtensionJobDispatcher> {
        let dispatcher = self.dispatcher(activation, commands);
        Arc::new(match crash {
            Some(hook) => dispatcher.with_crash_hook(hook),
            None => dispatcher,
        })
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

/// When `job` has just ended, log `contribution.job.completed` saying how,
/// in the transaction that ended it, with the anchors and cause it was
/// started with. Nothing for a job still queued or running.
pub(crate) fn announce_tx(
    conn: &rusqlite::Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    job: &ExtensionJob,
) -> Result<(), oxplow_domain::DomainError> {
    use oxplow_domain::events::schema::{
        ContributionJobCompleted, ContributionJobCompletedV1, JobOutcome,
    };
    let outcome = match job.state {
        JobState::Succeeded => JobOutcome::Succeeded,
        JobState::Failed => JobOutcome::Failed,
        JobState::Cancelled => JobOutcome::Cancelled,
        JobState::Interrupted => JobOutcome::Interrupted,
        JobState::Queued | JobState::Running => return Ok(()),
    };
    let context = extension_job_store::get_context_tx(conn, &job.id)
        .map_err(|e| oxplow_domain::DomainError::Invalid(e.to_string()))?;
    let payload = ContributionJobCompletedV1 {
        job_run: oxplow_domain::refs::build::job_run_ref(job.id.as_str()),
        job: job.owner.contribution.to_string(),
        version: job.owner.contract_version,
        attempt: job.attempt,
        fence: job.fence,
        outcome,
    };
    let events = oxplow_db::EventCtx::system(vocabulary, "extension-jobs");
    let mut event = events
        .typed::<ContributionJobCompleted>(&payload)
        .with_anchors(context.anchors().clone())
        .with_subject(context.subject().iter().cloned());
    if let Some(cause) = context.cause() {
        event = event.with_cause(cause.clone());
    }
    events.append(conn, &event)?;
    Ok(())
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
/// The run a `job_run:<id>` ref names.
fn job_id(job_run: &str) -> Result<JobId, CommandError> {
    let id = oxplow_domain::refs::build::id_of(job_run, oxplow_domain::refs::kind::JOB_RUN)
        .map_err(|e| invalid(&e.to_string()))?;
    if id.len() > 64 {
        return Err(invalid("invalid job run identity"));
    }
    serde_json::from_value(Value::String(id)).map_err(|_| invalid("invalid job run identity"))
}

fn job_run_ref(id: &JobId) -> String {
    oxplow_domain::refs::build::job_run_ref(id.as_str())
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
/// `request` with its `role` as the job's extension means it: a built-in
/// role, or the bare id of one of its own `ai_profiles`, stored qualified
/// (`<extension>/<id>`) so the run resolves it without the manifest.
fn qualify_role(
    request: &Value,
    extension: &str,
    profiles: &[String],
) -> Result<Value, CommandError> {
    let refused = || {
        let declared = if profiles.is_empty() {
            "it declares none".to_string()
        } else {
            profiles
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        invalid(&format!(
            "`role` is `main`, `summarize`, `decide` or one of this extension's `ai_profiles` \
             ({declared})"
        ))
    };
    let mut request = request.clone();
    let Some(role) = request.get_mut("role") else {
        return Ok(request);
    };
    let name = role.as_str().ok_or_else(refused)?;
    let qualified = oxplow_ai::config::Role::parse_in(extension, name).map_err(|_| refused())?;
    if let oxplow_ai::config::Role::Profile { name, .. } = &qualified {
        if !profiles.contains(name) {
            return Err(refused());
        }
    }
    *role = Value::String(qualified.to_string());
    Ok(request)
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
