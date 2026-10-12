//! Prepared provider instances for one catalog generation. This stages
//! processes and adapters; publication is coordinated by the app host.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use oxplow_config::{ExtensionInstanceConfig, OxplowConfig};
use oxplow_domain::vocabulary::Vocabulary;

use super::{Instance, Live, PreparedProviderCapability, ProviderRegistry, Scope};
use crate::capabilities::{Implementation, Source};
use crate::extension_catalog::CatalogSnapshot;
use crate::vocabulary_reactor::VocabularyErrors;

struct Staged {
    instance: Arc<Instance>,
    capability: Option<PreparedProviderCapability>,
    live: Option<Live>,
    epoch: u64,
    published: bool,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        self.instance
            .stopped
            .store(true, std::sync::atomic::Ordering::Release);
        drop(self.live.take());
        if let Ok(mut live) = self.instance.live.try_lock() {
            drop(live.take());
        }
    }
}

/// All provider instances and adapters staged for one catalog generation.
/// Dropping an unpublished value marks its candidates stopped and drops
/// their processes as a last resort.
pub struct PreparedProviderGeneration {
    generation: String,
    source_generation: String,
    configured: BTreeMap<String, (Scope, ExtensionInstanceConfig)>,
    global_mtime: Option<SystemTime>,
    base: BTreeMap<String, Arc<Instance>>,
    staged: Vec<Staged>,
    _reconciling: tokio::sync::OwnedMutexGuard<()>,
}

impl PreparedProviderGeneration {
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Capability metadata derived from the already validated adapters.
    pub fn implementations(&self) -> Vec<Implementation> {
        self.staged
            .iter()
            .map(|staged| Implementation {
                capability: staged.instance.spec.capability.clone(),
                id: staged.instance.id.clone(),
                title: staged.instance.name.clone(),
                extension: Some(staged.instance.ext.name.clone()),
                source: Source::External,
                features: staged
                    .capability
                    .as_ref()
                    .map(|capability| capability.features().clone())
                    .unwrap_or_default(),
                fields: serde_json::to_value(&staged.instance.spec.fields).unwrap_or_default(),
                id_pattern: staged.instance.spec.id_pattern.clone(),
                config: serde_json::json!({}),
            })
            .collect()
    }
}

/// The host maps swapped after the candidate database transaction commits.
pub struct ProviderGenerationPublication {
    retired: Vec<(Arc<Instance>, Option<Live>)>,
    published: Vec<Arc<Instance>>,
    registry: std::sync::Weak<ProviderRegistry>,
}

impl ProviderGenerationPublication {
    /// Record successful starts, schedule their initial sync and end replaced
    /// processes outside the publication transaction.
    pub async fn finish(mut self) {
        if let Some(registry) = self.registry.upgrade() {
            for instance in &self.published {
                let current = {
                    let running = registry.running.lock().await;
                    let current = running
                        .get(&instance.name)
                        .is_some_and(|active| Arc::ptr_eq(active, instance))
                        && !instance.is_stopped();
                    if current {
                        let mut health = registry.health.lock();
                        let state = health.entry(instance.name.clone()).or_insert_with(|| {
                            super::InstanceHealth::new(super::InstanceState::Ready)
                        });
                        state.state = super::InstanceState::Ready;
                        state.consecutive_failures = 0;
                        state.last_ok_at = Some(super::now());
                    }
                    current
                };
                if !current || !is_current(&registry, instance).await {
                    continue;
                }
                if let Err(error) = registry
                    .contribution_health
                    .succeeded(&super::contribution_key(&instance.name), None)
                    .await
                {
                    tracing::warn!(instance = %instance.name, error = %error, "recording its health failed");
                }
                if !is_current(&registry, instance).await {
                    continue;
                }
                let me = self.registry.clone();
                let instance = instance.clone();
                tokio::spawn(async move {
                    if let Some(registry) = me.upgrade() {
                        if is_current(&registry, &instance).await {
                            registry.sync_started(&instance.name).await;
                        }
                    }
                });
            }
        }
        for (instance, live) in std::mem::take(&mut self.retired) {
            let live = match live {
                Some(live) => Some(live),
                None => instance.live.lock().await.take(),
            };
            instance.end(live).await;
        }
    }
}

async fn is_current(registry: &ProviderRegistry, instance: &Arc<Instance>) -> bool {
    let running = registry.running.lock().await;
    running
        .get(&instance.name)
        .is_some_and(|active| Arc::ptr_eq(active, instance))
        && !instance.is_stopped()
}

/// A reserved running map and prepared provider candidate. The running
/// mutex is acquired before database publication and borrowed here so a
/// retry can retain the same exclusion guard.
pub struct ProviderGenerationReservation<'a> {
    registry: &'a ProviderRegistry,
    prepared: &'a mut PreparedProviderGeneration,
    running: &'a mut tokio::sync::OwnedMutexGuard<BTreeMap<String, Arc<Instance>>>,
    withdrawn: bool,
}

impl ProviderRegistry {
    /// Hold the running-map writer across candidate publication. Acquire this
    /// before entering the database write boundary.
    pub async fn reserve_running(
        &self,
    ) -> tokio::sync::OwnedMutexGuard<BTreeMap<String, Arc<Instance>>> {
        self.running.clone().lock_owned().await
    }

    /// Prepare all enabled, non-disabled instances against one immutable
    /// catalog snapshot. Startup is strict: any failed candidate rejects all.
    pub async fn prepare_generation(
        &self,
        snapshot: &CatalogSnapshot,
    ) -> Result<PreparedProviderGeneration, VocabularyErrors> {
        self.prepare_generation_inner(snapshot, true).await
    }

    /// A repairable startup baseline captures config/running fences but starts
    /// no external instances and publishes no external capability authority.
    pub(crate) async fn prepare_core_generation(
        &self,
        snapshot: &CatalogSnapshot,
    ) -> Result<PreparedProviderGeneration, VocabularyErrors> {
        self.prepare_generation_inner(snapshot, false).await
    }

    async fn prepare_generation_inner(
        &self,
        snapshot: &CatalogSnapshot,
        start_instances: bool,
    ) -> Result<PreparedProviderGeneration, VocabularyErrors> {
        let reconciling = self.reconciling.clone().lock_owned().await;
        let configured = self.scoped_instances();
        let global_mtime = self.global_mtime();
        let base = self.running.lock().await.clone();
        let mut staged = Vec::new();
        let mut ids = BTreeSet::new();
        let declared = crate::capabilities::declared_by(snapshot.extensions());

        for (name, (scope, config)) in &configured {
            if !start_instances || !config.enabled {
                continue;
            }
            let epoch = self.disable_epoch(name);
            let resolved = match Self::resolve_in(
                snapshot.extensions(),
                name,
                config.provider.as_deref(),
                *scope,
            ) {
                Ok(resolved) => resolved,
                Err(_) => {
                    cleanup(&mut staged).await;
                    return Err(error(name, "provider configuration could not be resolved"));
                }
            };
            match self.disabled_reason(name).await {
                Ok(None) => {}
                Ok(Some(_)) => continue,
                Err(_) => {
                    cleanup(&mut staged).await;
                    return Err(error(name, "provider disabled state could not be read"));
                }
            }
            if !ids.insert(resolved.id.clone()) {
                cleanup(&mut staged).await;
                return Err(error(name, "provider instance id is duplicated"));
            }
            if self.hosts.contains_key(resolved.spec.capability.as_str())
                && declared.iter().any(|implementation| {
                    implementation.capability == resolved.spec.capability
                        && implementation.id == resolved.id
                })
            {
                cleanup(&mut staged).await;
                return Err(error(
                    name,
                    "provider capability id conflicts with a declared implementation",
                ));
            }
            let instance = match self
                .instance_in_snapshot(
                    snapshot,
                    &resolved.ext,
                    &resolved.spec,
                    &resolved.id,
                    resolved.scope,
                    config.config.clone(),
                )
                .await
            {
                Ok(instance) => instance,
                Err(_) => {
                    cleanup(&mut staged).await;
                    return Err(error(name, "provider candidate could not be constructed"));
                }
            };
            let capability = match self
                .host_of(&instance)
                .and_then(|host| host.prepare(&instance))
            {
                Ok(capability) => capability,
                Err(_) => {
                    cleanup(&mut staged).await;
                    return Err(error(name, "provider capability could not be prepared"));
                }
            };
            match instance.start().await {
                Ok(live) => staged.push(Staged {
                    instance,
                    capability: Some(capability),
                    live: Some(live),
                    epoch,
                    published: false,
                }),
                Err(_) => {
                    cleanup(&mut staged).await;
                    return Err(error(name, "provider candidate failed to start"));
                }
            }
        }

        if self
            .deps
            .catalog
            .snapshot(&self.deps.project_dir)
            .generation()
            != snapshot.source_generation()
        {
            cleanup(&mut staged).await;
            return Err(host_error("provider catalog changed during preparation"));
        }

        Ok(PreparedProviderGeneration {
            generation: snapshot.generation().to_string(),
            source_generation: snapshot.source_generation().to_string(),
            configured,
            global_mtime,
            base,
            staged,
            _reconciling: reconciling,
        })
    }

    /// Reserve provider publication after acquiring `running` before the
    /// database transaction. Source/consent checks do bounded host file reads;
    /// this method starts no process and performs no network work.
    pub fn reserve_generation<'a>(
        &'a self,
        prepared: &'a mut PreparedProviderGeneration,
        running: &'a mut tokio::sync::OwnedMutexGuard<BTreeMap<String, Arc<Instance>>>,
        config: &OxplowConfig,
    ) -> Result<ProviderGenerationReservation<'a>, VocabularyErrors> {
        let current = &**running;
        let same_base = current.len() == prepared.base.len()
            && prepared
                .base
                .iter()
                .all(|(name, base)| current.get(name).is_some_and(|now| Arc::ptr_eq(now, base)));
        if !same_base {
            return Err(host_error(
                "running provider instances changed during preparation",
            ));
        }
        if prepared
            .staged
            .iter()
            .any(|staged| self.disable_epoch(&staged.instance.name) != staged.epoch)
        {
            return Err(host_error("provider enablement changed during preparation"));
        }
        if self
            .deps
            .catalog
            .snapshot(&self.deps.project_dir)
            .generation()
            != prepared.source_generation
            || self.global_mtime() != prepared.global_mtime
        {
            return Err(host_error("provider source changed during preparation"));
        }

        let mut configured: BTreeMap<_, _> = self
            .global_instances()
            .into_iter()
            .map(|(name, config)| (name, (Scope::Global, config)))
            .collect();
        configured.extend(
            config
                .extension_instances
                .iter()
                .map(|(name, config)| (name.clone(), (Scope::Project, config.clone()))),
        );
        if configured != prepared.configured {
            return Err(host_error(
                "provider configuration changed during preparation",
            ));
        }

        Ok(ProviderGenerationReservation {
            registry: self,
            prepared,
            running,
            withdrawn: false,
        })
    }
}

impl ProviderGenerationReservation<'_> {
    /// Validate provider event declarations against the candidate vocabulary
    /// before the candidate database transaction commits.
    pub fn validate_vocabulary(&self, vocabulary: &Vocabulary) -> Result<(), VocabularyErrors> {
        for staged in &self.prepared.staged {
            for declaration in &staged.instance.declared.event_types {
                if vocabulary.schema(&declaration.event_type, declaration.v)
                    != Some(&declaration.schema)
                {
                    return Err(error(
                        &staged.instance.name,
                        "provider event declaration is not in the candidate vocabulary",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Recheck staged liveness and source fences immediately before the
    /// caller commits its database transaction.
    pub fn validate_live(&self) -> Result<(), VocabularyErrors> {
        for staged in &self.prepared.staged {
            let valid = staged
                .live
                .as_ref()
                .is_some_and(|live| !live.conn.peer.is_closed())
                && staged.instance.host_calls.authority_current();
            if !valid {
                return Err(error(
                    &staged.instance.name,
                    "provider candidate is no longer live or authorized",
                ));
            }
            if self.registry.disable_epoch(&staged.instance.name) != staged.epoch {
                return Err(error(
                    &staged.instance.name,
                    "provider enablement changed during preparation",
                ));
            }
        }
        if self
            .registry
            .deps
            .catalog
            .snapshot(&self.registry.deps.project_dir)
            .generation()
            != self.prepared.source_generation
            || self.registry.global_mtime() != self.prepared.global_mtime
        {
            return Err(host_error("provider source changed during preparation"));
        }
        Ok(())
    }

    /// Remove old adapters after the database commit and before publishing
    /// their replacements into native registries.
    pub fn withdraw_maps(&mut self) {
        if self.withdrawn {
            return;
        }
        for instance in self.running.values() {
            if let Ok(host) = self.registry.host_of(instance) {
                host.retire(&instance.id);
            }
        }
        self.withdrawn = true;
    }

    /// Swap adapters and the running map after the candidate database
    /// transaction commits. Database/capability metadata are coordinated by
    /// the caller; this performs only synchronous native map publication.
    pub fn apply(&mut self) -> ProviderGenerationPublication {
        self.withdraw_maps();
        let old = std::mem::take(&mut **self.running);
        let mut retired = Vec::with_capacity(old.len());
        for instance in old.into_values() {
            instance
                .stopped
                .store(true, std::sync::atomic::Ordering::Release);
            instance
                .admitted
                .store(false, std::sync::atomic::Ordering::Release);
            // Production live guards perform only short in-memory work; no
            // guard waits on a process or network request. Take ownership so
            // cancellation of the async finisher still drops the connection.
            let live = instance.live.blocking_lock().take();
            retired.push((instance, live));
        }

        let mut supervisors = Vec::with_capacity(self.prepared.staged.len());
        for staged in &mut self.prepared.staged {
            if let Some(capability) = staged.capability.take() {
                capability.publish();
            }
            let peer = staged.live.as_ref().map(|live| live.conn.peer.clone());
            let mut live = staged
                .instance
                .live
                .try_lock()
                .expect("a staged instance has no active live reader");
            *live = staged.live.take();
            drop(live);
            self.running
                .insert(staged.instance.name.clone(), staged.instance.clone());
            staged
                .instance
                .admitted
                .store(true, std::sync::atomic::Ordering::Release);
            staged.published = true;
            if let Some(peer) = peer {
                supervisors.push((staged.instance.clone(), peer));
            }
        }
        for (instance, peer) in supervisors {
            instance.supervise(peer);
        }
        self.registry.global.lock().reconciled_at = self.prepared.global_mtime;
        self.registry.reconciled.send_replace(true);
        ProviderGenerationPublication {
            retired,
            published: self
                .prepared
                .staged
                .iter()
                .map(|staged| staged.instance.clone())
                .collect(),
            registry: self.registry.me.clone(),
        }
    }
}

async fn cleanup(staged: &mut Vec<Staged>) {
    for candidate in staged.iter_mut() {
        candidate
            .instance
            .stopped
            .store(true, std::sync::atomic::Ordering::Release);
        candidate.instance.end(candidate.live.take()).await;
        candidate.instance.discard_unpublished().await;
    }
    staged.clear();
}

fn error(instance: &str, message: &str) -> VocabularyErrors {
    VocabularyErrors::from([(instance.to_string(), vec![message.to_string()])])
}

fn host_error(message: &str) -> VocabularyErrors {
    error("<host>", message)
}
