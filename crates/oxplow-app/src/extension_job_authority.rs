//! Fresh job authority, separate from durable scheduling provenance.
//!
//! Resolver implementations are trusted host policy. Extension JSON cannot
//! construct a permission, pick its actor or supply a consent guard.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use oxplow_domain::{
    contribution::Descriptor,
    extension_job::JobOwner,
    extension_job_context::{JobExecutionIdentity, JobInvocationContext},
    run::{Authority, Budget, ContributionRef, RunContext},
    stores::ThreadStore,
};

use crate::{
    extension_catalog::ExtensionCatalog,
    extension_job_operations::{JobOperationKey, JobOperationRegistry, JobOperations},
    run_context::LiveCheck,
};

#[derive(Debug, thiserror::Error)]
pub enum JobAuthorityError {
    #[error("job execution authority is unavailable")]
    Unavailable,
    #[error("job execution authority was refused")]
    Denied,
    #[error("job authority storage failed")]
    Storage(#[source] oxplow_domain::DomainError),
}

/// This is a host value, without a wire representation. Its live guard must
/// recheck the job's extension and scheduling-principal policy at sensitive
/// calls.
pub struct JobPermission {
    authority: Authority,
    live: LiveCheck,
}

impl JobPermission {
    pub fn new(authority: Authority, live: LiveCheck) -> Self {
        Self { authority, live }
    }
}

#[async_trait]
pub trait JobGrantResolver: Send + Sync {
    /// Resolve the job's present declaration and its extension's
    /// permissions; never revive a persisted grant or confirmation. Check
    /// the scheduling principal's current policy.
    async fn resolve(
        &self,
        owner: &JobOwner,
        descriptor: &Descriptor,
        provenance: &JobInvocationContext,
    ) -> Result<JobPermission, JobAuthorityError>;
}

/// The authority of a manifest-declared host job: its declaration as the
/// live catalog has it, within its extension's permissions. No request or
/// persisted provenance can choose an authority scope.
pub struct DeclaredJobGrants {
    catalog: Arc<ExtensionCatalog>,
    project: PathBuf,
}

impl DeclaredJobGrants {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        Self { catalog, project }
    }
}

#[async_trait]
impl JobGrantResolver for DeclaredJobGrants {
    async fn resolve(
        &self,
        owner: &JobOwner,
        descriptor: &Descriptor,
        provenance: &JobInvocationContext,
    ) -> Result<JobPermission, JobAuthorityError> {
        provenance
            .validate()
            .map_err(|_| JobAuthorityError::Denied)?;
        // This operation reads through ai.compute. Current principal policy
        // is the host's thread/stream check plus explicit model-use consent;
        // it acquires no generic command or workspace-changing admission.
        if descriptor.id.kind != oxplow_domain::contribution::ContributionKind::Job
            || descriptor.id != owner.contribution
            || !crate::extensions::jobs::needs_hold(&descriptor.needs)
        {
            return Err(JobAuthorityError::Denied);
        }
        let snapshot = self.catalog.live_snapshot(&self.project);
        let extension = snapshot
            .extensions()
            .iter()
            .find(|extension| {
                extension.enabled
                    && extension.name == descriptor.owner.extension
                    && extension.origin == descriptor.owner.origin
                    && extension.version == owner.package_version
            })
            .ok_or(JobAuthorityError::Unavailable)?;
        // Its declaration, as the live catalog has it, must still be this.
        if !extension.jobs.iter().any(|job| {
            job.id == owner.contribution.local
                && Some(job.version) == owner.contract_version
                && job.needs == descriptor.needs
        }) {
            return Err(JobAuthorityError::Unavailable);
        }
        let authority =
            crate::grants::authority(extension, crate::grants::script_request(&descriptor.needs));
        Ok(JobPermission::new(authority, Arc::new(|| Ok(()))))
    }
}

pub struct JobAuthorityHost {
    catalog: Arc<ExtensionCatalog>,
    project: PathBuf,
    registry: JobOperations,
    grants: Arc<dyn JobGrantResolver>,
    threads: Arc<dyn ThreadStore>,
    activation: Option<Arc<crate::extension_activation::ExtensionActivation>>,
}

#[derive(Clone)]
pub struct JobExecution {
    run: RunContext,
    live: LiveCheck,
    /// It declares a need that changes the workspace: its thread must be
    /// open.
    needs_thread: bool,
}

impl JobExecution {
    pub fn run(&self) -> &RunContext {
        &self.run
    }

    /// Actual completion may win a pending cancellation. This checks live
    /// ownership/grants independently of the cancellation flag; it does not
    /// authorize another host call or confirmation after cancellation.
    pub fn check_publication(&self, conn: &rusqlite::Connection) -> Result<(), JobAuthorityError> {
        (self.live)().map_err(|_| JobAuthorityError::Denied)?;
        match self.run.anchors().thread_id {
            Some(id) => {
                let thread = oxplow_db::thread_store::get_tx(conn, id)
                    .map_err(oxplow_db::map_sql_err)
                    .map_err(JobAuthorityError::Storage)?
                    .ok_or(JobAuthorityError::Denied)?;
                if self.run.anchors().stream_id != Some(thread.stream_id)
                    || (self.needs_thread && !thread.is_open())
                {
                    return Err(JobAuthorityError::Denied);
                }
            }
            None if self.needs_thread => return Err(JobAuthorityError::Denied),
            None => {}
        }
        Ok(())
    }
}

impl JobAuthorityHost {
    pub fn new(
        catalog: Arc<ExtensionCatalog>,
        project: PathBuf,
        registry: Arc<JobOperationRegistry>,
        grants: Arc<dyn JobGrantResolver>,
        threads: Arc<dyn ThreadStore>,
    ) -> Self {
        let project = catalog.project_of(&project).to_path_buf();
        Self {
            catalog,
            project,
            registry: JobOperations::new(registry),
            grants,
            threads,
            activation: None,
        }
    }

    pub fn registry(&self) -> Arc<JobOperationRegistry> {
        self.registry.snapshot()
    }

    pub fn with_registry_handle(mut self, registry: JobOperations) -> Self {
        self.registry = registry;
        self
    }

    /// Trusted host wiring supplies this project's activation owner. Durable
    /// scheduling provenance never supplies or retains an execution pin.
    pub fn with_activation(
        mut self,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Self {
        self.activation = Some(activation);
        self
    }

    pub(crate) async fn read_generation<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, JobAuthorityError> {
        match &self.activation {
            Some(activation) => activation
                .read(future)
                .await
                .map_err(|_| JobAuthorityError::Unavailable),
            None => Ok(future.await),
        }
    }

    pub async fn authorize(
        &self,
        job: &oxplow_db::extension_job_store::StoredExtensionJob,
        identity: &JobExecutionIdentity,
        provenance: &JobInvocationContext,
    ) -> Result<JobExecution, JobAuthorityError> {
        self.read_generation(self.authorize_unpinned(job, identity, provenance))
            .await?
    }

    async fn authorize_unpinned(
        &self,
        job: &oxplow_db::extension_job_store::StoredExtensionJob,
        identity: &JobExecutionIdentity,
        provenance: &JobInvocationContext,
    ) -> Result<JobExecution, JobAuthorityError> {
        provenance
            .validate()
            .map_err(|_| JobAuthorityError::Denied)?;
        job.job.validate().map_err(|_| JobAuthorityError::Denied)?;
        if identity.job_id() != &job.job.id || provenance.originating_run_id() != &job.job.run_id {
            return Err(JobAuthorityError::Denied);
        }
        let token = identity.attempt_fence();
        let token_matches = match job.job.state {
            oxplow_domain::extension_job::JobState::Queued => {
                job.job.attempt.checked_add(1) == Some(token.attempt)
                    && job.job.fence.checked_add(1) == Some(token.fence)
            }
            oxplow_domain::extension_job::JobState::Running => {
                job.job.attempt == token.attempt
                    && job.job.fence == token.fence
                    && job
                        .job
                        .lease_until_ms
                        .is_some_and(|lease| lease > oxplow_domain::Timestamp::now().unix_ms())
            }
            _ => false,
        };
        if !token_matches || job.job.cancellation_requested_at_ms.is_some() {
            return Err(JobAuthorityError::Denied);
        }
        let operation = self
            .registry
            .snapshot()
            .find(&job.job.owner, &job.operation)
            .map_err(|_| JobAuthorityError::Unavailable)?;
        if operation.replay_policy() != job.job.replay_policy {
            return Err(JobAuthorityError::Denied);
        }
        let descriptor = current_descriptor(&self.catalog, &self.project, &job.job.owner)?;
        let needs_thread = descriptor.needs.iter().any(|need| {
            oxplow_domain::scope::scope(oxplow_domain::service::need_scope(need))
                .is_some_and(|scope| scope.access.needs_thread())
        });
        match provenance.anchors().thread_id {
            Some(id) => {
                let thread = self
                    .threads
                    .get(&id)
                    .await
                    .map_err(JobAuthorityError::Storage)?
                    .ok_or(JobAuthorityError::Denied)?;
                if provenance.anchors().stream_id != Some(thread.stream_id)
                    || (needs_thread && !thread.is_open())
                {
                    return Err(JobAuthorityError::Denied);
                }
            }
            None if needs_thread => return Err(JobAuthorityError::Denied),
            None => {}
        }
        let permission = self
            .grants
            .resolve(&job.job.owner, &descriptor, provenance)
            .await?;
        // This initial dispatcher supports declared host services, not raw
        // process authority. Such operations require their own consent host.
        // A job that asks a model needs its extension's `ai` permission.
        if permission.authority.declared.scopes.contains("ai.compute")
            && !permission.authority.grant.permissions.ai
        {
            return Err(JobAuthorityError::Denied);
        }
        let (catalog, project, owner, approved) = (
            self.catalog.clone(),
            self.project.clone(),
            job.job.owner.clone(),
            permission.live,
        );
        let live: LiveCheck = Arc::new(move || {
            current_descriptor(&catalog, &project, &owner)
                .map_err(|_| "job ownership is no longer current".to_string())?;
            approved().map_err(|_| "the job's policy no longer allows it".to_string())
        });
        live().map_err(|_| JobAuthorityError::Denied)?;
        let run = RunContext::job(
            ContributionRef {
                id: job.job.owner.contribution.clone(),
                version: job.job.owner.package_version.clone(),
                // A job belongs to its extension's version, not to one
                // catalog generation: `live` checks that version is current.
                generation: None,
            },
            identity,
            provenance,
        )
        .map_err(|_| JobAuthorityError::Denied)?
        .authorized(permission.authority)
        .guarded(live.clone())
        // A job's own `timeout` (at most 10 min) fails it first; this is
        // the run's outer bound.
        .within(Budget::default().with_ceiling(Duration::from_secs(
            u64::from(crate::extensions::jobs::MAX_TIMEOUT_SECS) + 60,
        )));
        // No generation pin: a job runs for minutes and an activation must
        // not wait for it. Each tool call and the publication pin for
        // themselves, and `live` checks the job's own extension then.
        Ok(JobExecution {
            run,
            live,
            needs_thread,
        })
    }
}

fn current_descriptor(
    catalog: &ExtensionCatalog,
    project: &std::path::Path,
    owner: &JobOwner,
) -> Result<Descriptor, JobAuthorityError> {
    JobOperationKey::new(owner.clone(), "authority".into())
        .map_err(|_| JobAuthorityError::Denied)?;
    let snapshot = catalog.live_snapshot(project);
    let extension = crate::extensions::owner(snapshot.extensions(), &owner.contribution.extension)
        .filter(|extension| extension.enabled && extension.version == owner.package_version)
        .ok_or(JobAuthorityError::Unavailable)?;
    extension
        .contributions
        .iter()
        .find(|descriptor| {
            descriptor.id == owner.contribution
                && descriptor.contract_version == owner.contract_version
        })
        .cloned()
        .ok_or(JobAuthorityError::Unavailable)
}
