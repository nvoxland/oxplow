//! Prepared provider instances for one catalog generation. This stages
//! processes and adapters; publication is coordinated by the app host.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use oxplow_config::{ExtensionInstanceConfig, OxplowConfig};
use oxplow_domain::vocabulary::Vocabulary;

use super::{
    HostError, Instance, InstanceState, Live, PreparedProviderCapability, ProviderRegistry, Scope,
};
use crate::capabilities::{Implementation, Source};
use crate::extension_catalog::CatalogSnapshot;
use crate::vocabulary_reactor::VocabularyErrors;

struct Staged {
    instance: Arc<Instance>,
    capability: Option<PreparedProviderCapability>,
    live: Option<Live>,
    epoch: u64,
    published: bool,
    /// Its start failed the way a start may (the process won't come up
    /// yet): it is published without a connection, failing, and its next
    /// call restarts it with backoff.
    start_failure: Option<String>,
    /// The running instance carried over unchanged: its process is the
    /// generation's too, so nothing of it is stopped if this isn't published.
    reused: bool,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.published || self.reused {
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
    /// What instances left out of the generation read as, set when it is
    /// published (a retired instance's late failure must not override it).
    states: Vec<(String, InstanceState)>,
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
                icon: None,
                config: serde_json::json!({}),
            })
            .collect()
    }
}

/// The host maps swapped after the candidate database transaction commits.
pub struct ProviderGenerationPublication {
    retired: Vec<(Arc<Instance>, Option<Live>)>,
    published: Vec<Arc<Instance>>,
    start_failures: Vec<(Arc<Instance>, String)>,
    registry: std::sync::Weak<ProviderRegistry>,
}

impl ProviderGenerationPublication {
    /// Record successful starts, schedule their initial sync and end replaced
    /// processes outside the publication transaction.
    pub async fn finish(mut self) {
        if let Some(registry) = self.registry.upgrade() {
            for (instance, error) in std::mem::take(&mut self.start_failures) {
                registry
                    .failed(&instance.name.clone(), Some(&instance), error)
                    .await;
            }
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

        // One instance that can't run never holds back the others: it is
        // left out and its state says why (the way the instance's
        // `health` reads it), so the generation activates without it.
        let mut states: Vec<(String, InstanceState)> = Vec::new();
        for (name, (scope, config)) in &configured {
            if !start_instances {
                continue;
            }
            let resolved = match Self::resolve_in(
                snapshot.extensions(),
                name,
                config.provider.as_deref(),
                *scope,
            ) {
                Ok(resolved) => resolved,
                Err(why) => {
                    states.push((
                        name.clone(),
                        InstanceState::Missing {
                            reason: format!("`{name}`: {why}"),
                        },
                    ));
                    continue;
                }
            };
            if !config.enabled {
                states.push((name.clone(), InstanceState::Off));
                continue;
            }
            let epoch = self.disable_epoch(name);
            match self.disabled_reason(name).await {
                Ok(None) => {}
                Ok(Some(reason)) => {
                    states.push((name.clone(), InstanceState::Disabled { reason }));
                    continue;
                }
                // Not knowing is not a yes: it stays off.
                Err(e) => {
                    states.push((
                        name.clone(),
                        InstanceState::Failing {
                            errors: vec![format!("couldn't read whether it was disabled: {e}")],
                        },
                    ));
                    continue;
                }
            }
            let mut refused = |reason: &str| {
                states.push((
                    name.clone(),
                    InstanceState::Refused {
                        reason: reason.to_string(),
                    },
                ));
            };
            if !ids.insert(resolved.id.clone()) {
                refused("provider instance id is duplicated");
                continue;
            }
            if self.hosts.contains_key(resolved.spec.capability.as_str())
                && declared.iter().any(|implementation| {
                    implementation.capability == resolved.spec.capability
                        && implementation.id == resolved.id
                })
            {
                refused("provider capability id conflicts with a declared implementation");
                continue;
            }
            // Its id is a command namespace and a provider id: another
            // owner's, it is refused (its row says so).
            if let Some(owner) = self
                .bus
                .upgrade()
                .and_then(|bus| bus.namespace_owner(&resolved.id))
                .filter(|owner| *owner != oxplow_domain::refs::build::provider_source(name))
            {
                refused(&format!(
                    "the command namespace `{}` is already {owner}'s — give the instance another id",
                    resolved.id
                ));
                continue;
            }
            if !base.contains_key(name) && self.hosts.values().any(|h| h.has(&resolved.id)) {
                refused(&format!(
                    "`{}` is already a provider — give the instance another id",
                    resolved.id
                ));
                continue;
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
                Err(e) => {
                    self.start_failed(name, None, e).await;
                    continue;
                }
            };
            // A running instance this generation doesn't change keeps its
            // process: the same program, config, approval and declarations.
            // One whose approval no longer covers its program (an edit
            // re-approved, a revocation) is replaced here: reusing it would
            // fail `validate_live` and reject the whole generation.
            let unchanged = base.get(name).filter(|current| {
                !current.is_stopped()
                    && current.host_calls.authority_current()
                    && current.scope == instance.scope
                    && current.config == instance.config
                    && current.spec == instance.spec
                    && same_extension(&current.ext, &instance.ext)
                    && current.declared == instance.declared
                    && current.is_connected()
            });
            let (instance, reused) = match unchanged {
                Some(current) => (current.clone(), true),
                None => (instance, false),
            };
            let capability = match self
                .host_of(&instance)
                .and_then(|host| host.prepare(&instance))
            {
                Ok(capability) => capability,
                Err(reason) => {
                    refused(&reason);
                    continue;
                }
            };
            if reused {
                staged.push(Staged {
                    instance,
                    capability: Some(capability),
                    live: None,
                    epoch,
                    published: false,
                    start_failure: None,
                    reused: true,
                });
                continue;
            }
            self.set_state(name, InstanceState::Checking);
            match instance.start().await {
                Ok(live) => staged.push(Staged {
                    instance,
                    capability: Some(capability),
                    live: Some(live),
                    epoch,
                    published: false,
                    start_failure: None,
                    reused: false,
                }),
                // It may come up: enabled and failing, its next call
                // restarts it with backoff.
                Err(e @ HostError::Failed { .. }) => staged.push(Staged {
                    instance,
                    capability: Some(capability),
                    live: None,
                    epoch,
                    published: false,
                    start_failure: Some(e.to_string()),
                    reused: false,
                }),
                Err(e) => {
                    instance.discard_unpublished().await;
                    self.start_failed(name, None, e).await;
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
            states,
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
            let connected = if staged.reused {
                !staged.instance.is_stopped() && staged.instance.is_connected()
            } else {
                match &staged.live {
                    Some(live) => !live.conn.peer.is_closed(),
                    None => staged.start_failure.is_some(),
                }
            };
            let valid = connected && staged.instance.host_calls.authority_current();
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
        let kept: BTreeSet<String> = self
            .prepared
            .staged
            .iter()
            .filter(|staged| staged.reused)
            .map(|staged| staged.instance.name.clone())
            .collect();
        for instance in old.into_values() {
            if kept.contains(&instance.name) {
                continue;
            }
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
            if staged.reused {
                self.running
                    .insert(staged.instance.name.clone(), staged.instance.clone());
                staged.published = true;
                continue;
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
        for (name, state) in std::mem::take(&mut self.prepared.states) {
            self.registry.set_state(&name, state);
        }
        self.registry.global.lock().reconciled_at = self.prepared.global_mtime;
        self.registry.reconciled.send_replace(true);
        ProviderGenerationPublication {
            retired,
            published: self
                .prepared
                .staged
                .iter()
                .filter(|staged| staged.start_failure.is_none() && !staged.reused)
                .map(|staged| staged.instance.clone())
                .collect(),
            start_failures: self
                .prepared
                .staged
                .iter()
                .filter_map(|staged| {
                    staged
                        .start_failure
                        .clone()
                        .map(|error| (staged.instance.clone(), error))
                })
                .collect(),
            registry: self.registry.me.clone(),
        }
    }
}

/// The same extension as far as a running provider is concerned: its place
/// among the other extensions (the order they activate in) is not part of it.
fn same_extension(a: &crate::extensions::Extension, b: &crate::extensions::Extension) -> bool {
    let mut b = b.clone();
    b.public_contracts.activation_index = a.public_contracts.activation_index;
    *a == b
}

async fn cleanup(staged: &mut Vec<Staged>) {
    for candidate in staged.iter_mut().filter(|candidate| !candidate.reused) {
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
