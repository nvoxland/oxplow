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
    exec_consent::Gate,
    extension_catalog::ExtensionCatalog,
    extension_job_operations::{JobOperationKey, JobOperationRegistry, JobOperations},
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
/// recheck current consent and scheduling-principal policy at sensitive calls.
pub struct JobPermission {
    authority: Authority,
    live: Gate,
}

impl JobPermission {
    pub fn new(authority: Authority, live: Gate) -> Self {
        Self { authority, live }
    }
}

#[async_trait]
pub trait JobGrantResolver: Send + Sync {
    /// Resolve present grants and approval hashes; never revive a persisted
    /// grant or confirmation. Check the scheduling principal's current policy.
    async fn resolve(
        &self,
        owner: &JobOwner,
        descriptor: &Descriptor,
        provenance: &JobInvocationContext,
    ) -> Result<JobPermission, JobAuthorityError>;
}

/// Current consent for a manifest-declared host job. No request or persisted
/// provenance can choose an approval key, source hash or authority scope.
pub struct DeclaredJobGrants {
    catalog: Arc<ExtensionCatalog>,
    project: PathBuf,
    approvals: Arc<crate::exec_consent::ApprovalStore>,
}

impl DeclaredJobGrants {
    pub fn new(
        catalog: Arc<ExtensionCatalog>,
        project: PathBuf,
        approvals: Arc<crate::exec_consent::ApprovalStore>,
    ) -> Self {
        Self {
            catalog,
            project,
            approvals,
        }
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
        // it acquires no generic command or workspace writer admission.
        if descriptor.id.kind != oxplow_domain::contribution::ContributionKind::Job
            || descriptor.id != owner.contribution
            || !crate::extensions::jobs::needs_hold(&descriptor.needs)
        {
            return Err(JobAuthorityError::Denied);
        }
        let snapshot = self.catalog.runtime_snapshot(&self.project);
        if snapshot.generation() != owner.generation {
            return Err(JobAuthorityError::Unavailable);
        }
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
        let declaration = extension
            .jobs
            .iter()
            .find(|job| {
                job.id == owner.contribution.local
                    && Some(job.version) == owner.contract_version
                    && job.needs == descriptor.needs
            })
            .ok_or(JobAuthorityError::Unavailable)?;
        let program = crate::exec_consent::job_program(extension, declaration);
        let gate = crate::exec_consent::script_gate(
            self.approvals.clone(),
            self.project.clone(),
            program.clone(),
        );
        gate().map_err(|_| JobAuthorityError::Denied)?;
        let authority = crate::grants::authority(
            &self.approvals,
            &program.key(),
            true,
            crate::grants::script_request(&descriptor.needs),
        );
        Ok(JobPermission::new(authority, gate))
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
    live: Gate,
    writer: bool,
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
                    || (self.writer && !thread.status.is_writer())
                {
                    return Err(JobAuthorityError::Denied);
                }
            }
            None if self.writer => return Err(JobAuthorityError::Denied),
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
        let writer = descriptor.needs.iter().any(|need| {
            oxplow_domain::scope::scope(oxplow_domain::service::need_scope(need))
                .is_some_and(|scope| scope.access.needs_writer())
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
                    || (writer && !thread.status.is_writer())
                {
                    return Err(JobAuthorityError::Denied);
                }
            }
            None if writer => return Err(JobAuthorityError::Denied),
            None => {}
        }
        let permission = self
            .grants
            .resolve(&job.job.owner, &descriptor, provenance)
            .await?;
        // This initial dispatcher supports declared host services, not raw
        // process authority. Such operations require their own consent host.
        let declared = crate::grants::script_request(&descriptor.needs);
        if !permission.authority.declared.widening(&declared).is_empty()
            || permission.authority.grant.as_ref().is_some_and(|grant| {
                !grant
                    .request
                    .widening(&permission.authority.declared)
                    .is_empty()
                    || grant.revoked_at.is_some()
            })
            || (permission.authority.declared.scopes.contains("ai.compute")
                && permission.authority.grant.is_none())
        {
            return Err(JobAuthorityError::Denied);
        }
        let (catalog, project, owner, approved) = (
            self.catalog.clone(),
            self.project.clone(),
            job.job.owner.clone(),
            permission.live,
        );
        let live: Gate = Arc::new(move || {
            current_descriptor(&catalog, &project, &owner)
                .map_err(|_| "job ownership is no longer current".to_string())?;
            approved().map_err(|_| "job approval or policy was revoked".to_string())
        });
        live().map_err(|_| JobAuthorityError::Denied)?;
        let run = RunContext::job(
            ContributionRef {
                id: job.job.owner.contribution.clone(),
                version: job.job.owner.package_version.clone(),
                generation: Some(job.job.owner.generation.clone()),
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
        let run = match &self.activation {
            Some(activation) => activation
                .pin_run(&run)
                .await
                .map_err(|_| JobAuthorityError::Unavailable)?,
            None => run,
        };
        Ok(JobExecution { run, live, writer })
    }
}

fn current_descriptor(
    catalog: &ExtensionCatalog,
    project: &std::path::Path,
    owner: &JobOwner,
) -> Result<Descriptor, JobAuthorityError> {
    JobOperationKey::new(owner.clone(), "authority".into())
        .map_err(|_| JobAuthorityError::Denied)?;
    let snapshot = catalog.runtime_snapshot(project);
    if snapshot.generation() != owner.generation {
        return Err(JobAuthorityError::Unavailable);
    }
    let extension = snapshot
        .extensions()
        .iter()
        .find(|extension| {
            extension.name == owner.contribution.extension && extension.origin == "bundled"
        })
        .or_else(|| {
            snapshot
                .extensions()
                .iter()
                .find(|extension| extension.name == owner.contribution.extension)
        })
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
