//! Native runtime adapters prepared from one extension catalog candidate.
//! Preparation never changes live registries, starts a process or calls AI.

use crate::{extension_catalog::CatalogSnapshot, vocabulary_reactor::VocabularyErrors, Services};

pub struct PreparedRuntimeAdapters {
    generation: String,
    work_items: crate::work_items::PreparedWorkItems,
    harnesses: crate::harnesses::PreparedHarnesses,
    policies: crate::effort_policy::PreparedEffortPolicies,
    knowledge: crate::knowledge::PreparedKnowledge,
    snapshots: crate::snapshots::PreparedSnapshots,
    models: crate::ai_service::PreparedModelProviders,
    jobs: std::sync::Arc<crate::extension_job_operations::JobOperationRegistry>,
}

impl PreparedRuntimeAdapters {
    pub fn prepare(svc: &Services, snapshot: &CatalogSnapshot) -> Result<Self, VocabularyErrors> {
        let load_errors: VocabularyErrors = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && !extension.errors.is_empty())
            .map(|extension| (extension.name.clone(), extension.errors.clone()))
            .collect();
        if !load_errors.is_empty() {
            return Err(load_errors);
        }
        let declared = crate::capabilities::declared_by(snapshot.extensions());
        let jobs = svc.jobs.prepare_registry(snapshot)?;
        let mut errors = VocabularyErrors::new();
        let models = crate::ai_service::prepare_built_ins(
            &declared,
            &svc.approvals,
            &svc.layout.project_dir,
        )
        .map_err(|failures| {
            for (id, message) in failures {
                errors.insert(format!("<ai:{id}>"), vec![message]);
            }
        });
        if !errors.is_empty() {
            return Err(errors);
        }
        let trees = std::sync::Arc::new(svc.metrics.clone())
            as std::sync::Arc<dyn crate::services::TreeSource>;
        Ok(Self {
            generation: snapshot.generation().to_string(),
            work_items: crate::work_items::prepare_built_ins(&declared, &svc.db),
            harnesses: crate::harnesses::prepare_built_ins(&declared),
            policies: crate::effort_policy::prepare_built_ins(
                &declared,
                &svc.sql,
                &trees,
                &svc.approvals,
                &svc.layout.project_dir,
            ),
            knowledge: crate::knowledge::prepare_built_ins(&declared, &svc.layout.project_dir),
            snapshots: crate::snapshots::prepare_built_ins(
                &declared,
                &svc.snapshot_captures,
                &svc.snapshot_files(),
            ),
            models: models.expect("all adapter errors were rejected"),
            jobs,
        })
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Infallible map publication without resolving active selections. The
    /// coordinator must exclude readers and competing writers across these
    /// swaps and the database commit. This method alone does not establish a
    /// coherent activation boundary.
    pub(crate) fn publish_maps(self, svc: &Services) {
        self.work_items.publish(&svc.work_items);
        self.harnesses.publish(&svc.harnesses);
        self.policies.publish(&svc.effort_policies);
        self.knowledge.publish_maps(&svc.knowledge, &svc.wiki);
        self.snapshots
            .publish(&svc.snapshots, &svc.snapshot_captures);
        self.models.publish(svc.ai.providers());
        svc.jobs.publish_registry(self.jobs);
    }

    /// Follow the newly published active selections. Capability and config
    /// reservations must be released before calling this method because the
    /// active callbacks read both registries; keep readers excluded by the
    /// outer runtime publication gate until it returns.
    pub(crate) fn follow_selections(svc: &Services) {
        svc.wiki.follow(&svc.knowledge.active_id());
        crate::snapshots::apply_active_policy(&svc.snapshots, &svc.snapshot_captures);
    }

    /// Follow explicit selections from the prepared capability candidate,
    /// without resolving through live capability/config callbacks.
    pub(crate) fn follow_selected(svc: &Services, knowledge: &str, snapshots: &str) {
        svc.wiki.follow(knowledge);
        crate::snapshots::apply_selected_policy(&svc.snapshots, &svc.snapshot_captures, snapshots);
    }

    /// Publish maps and then follow the selected wiki and snapshot providers.
    /// Call only when no capability or config reservation is held.
    pub fn publish(self, svc: &Services) {
        self.publish_maps(svc);
        Self::follow_selections(svc);
    }
}
