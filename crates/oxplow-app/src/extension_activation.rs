//! Publication and reader boundary for prepared extension generations.
//! Boot adopts this boundary before exposing extension runtime consumers.

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oxplow_db::TxError;
use oxplow_domain::{run::RunContext, DomainError, Timestamp};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::extension_catalog::{CatalogSnapshot, LiveChanges};
use crate::vocabulary_reactor::VocabularyErrors;
use crate::Services;

#[derive(Default)]
struct GenerationState {
    active: Option<Arc<CatalogSnapshot>>,
    unavailable: bool,
    entities: crate::extension_models::EntitySignature,
    /// The configuration the published models were compiled against.
    configs: crate::extension_generation::ConfigSignature,
}

/// How often a publication waiting on readers says who they are.
const WAITING_LOG_EVERY: Duration = Duration::from_secs(10);

/// A generation reader as a waiting publication sees it: what holds it
/// (the run, or the reading future's type) and for how long.
#[derive(Debug, Clone)]
pub struct Reader {
    pub held_by: String,
    pub held_for: Duration,
}

/// The readers holding a generation now, each until its pin is dropped.
#[derive(Default)]
struct Readers {
    next: AtomicU64,
    held: parking_lot::Mutex<HashMap<u64, (Cow<'static, str>, Instant)>>,
}

/// One entry of [`Readers`], removed when the last clone of its pin goes.
struct Held {
    readers: Arc<Readers>,
    id: u64,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.readers.held.lock().remove(&self.id);
    }
}

/// A host-only reader lifetime. Its private guard excludes publication until
/// the last nested run/query releases it. It conveys no service authority.
#[derive(Clone)]
pub struct GenerationPin {
    owner: Arc<RwLock<GenerationState>>,
    state: Arc<OwnedRwLockReadGuard<GenerationState>>,
    _held: Arc<Held>,
}

impl std::fmt::Debug for GenerationPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GenerationPin")
            .finish_non_exhaustive()
    }
}

impl GenerationPin {
    /// The activated catalog, or `None` before the first activation: the
    /// boundary then serves the current load.
    pub fn snapshot(&self) -> Option<&Arc<CatalogSnapshot>> {
        self.state.active.as_ref()
    }

    pub fn generation(&self) -> Option<&str> {
        self.snapshot().map(|snapshot| snapshot.generation())
    }
}

tokio::task_local! {
    static READERS: Vec<GenerationPin>;
}

/// One project's activation owner. Readers must enter `read` or retain a pin
/// in RunContext before looking up any generation-owned handler or metadata.
/// Other generation writers must be retired before adopting this coordinator.
#[derive(Default)]
pub struct ExtensionActivation {
    state: Arc<RwLock<GenerationState>>,
    activating: tokio::sync::Mutex<()>,
    requested: tokio::sync::Notify,
    spawned: std::sync::atomic::AtomicBool,
    /// Set once the first generation is published; before it the boundary
    /// serves the current load.
    activated: std::sync::atomic::AtomicBool,
    problems: parking_lot::Mutex<VocabularyErrors>,
    readers: Arc<Readers>,
    #[cfg(test)]
    panics: parking_lot::Mutex<Vec<(PanicPoint, u32)>>,
}

/// Where in a publication, past its database commit, a test makes the
/// worker panic.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PanicPoint {
    /// Right after the commit: no in-memory map has been swapped.
    AfterCommit,
    /// After the maps, catalog and providers are published, before the
    /// boundary shows the new generation.
    BeforeBoundary,
}

impl ExtensionActivation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a generation has been published: the owner of command,
    /// provider and model publication from then on.
    pub fn is_active(&self) -> bool {
        self.activated.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn request(&self) {
        self.requested.notify_one();
    }

    pub fn problems(&self) -> VocabularyErrors {
        self.problems.lock().clone()
    }

    /// The readers holding the generation now, longest-held first: what a
    /// publication waits for.
    pub fn readers(&self) -> Vec<Reader> {
        let mut readers: Vec<Reader> = self
            .readers
            .held
            .lock()
            .values()
            .map(|(held_by, since)| Reader {
                held_by: held_by.to_string(),
                held_for: since.elapsed(),
            })
            .collect();
        readers.sort_by_key(|reader| std::cmp::Reverse(reader.held_for));
        readers
    }

    /// The next publication panics once its database state has committed.
    #[cfg(test)]
    pub(crate) fn panic_once_after_commit(&self) {
        self.panic_at(PanicPoint::AfterCommit, 1);
    }

    /// The next `times` publications reaching `point` panic there.
    #[cfg(test)]
    pub(crate) fn panic_at(&self, point: PanicPoint, times: u32) {
        self.panics.lock().push((point, times));
    }

    #[cfg(test)]
    fn maybe_panic(&self, point: PanicPoint) {
        let mut panics = self.panics.lock();
        if let Some((_, left)) = panics
            .iter_mut()
            .find(|(at, left)| *at == point && *left > 0)
        {
            *left -= 1;
            drop(panics);
            panic!("injected panic at {point:?}");
        }
    }

    /// Data changes do not require provider restarts. Only a changed entity
    /// schema can make an otherwise unchanged model candidate compile.
    pub(crate) async fn request_entity_refresh(
        &self,
        db: &oxplow_db::Database,
    ) -> Result<(), DomainError> {
        let pin = self.pin().await?;
        let entities = db
            .read(crate::extension_models::entity_signature_tx)
            .await?;
        if pin.state.entities != entities {
            self.request();
        }
        Ok(())
    }

    /// An extension's stored configuration only changes what its models were
    /// compiled against when one of them reads it: republish when it has.
    pub(crate) async fn request_config_refresh(
        &self,
        db: &oxplow_db::Database,
    ) -> Result<(), DomainError> {
        let pin = self.pin().await?;
        let extensions = pin
            .snapshot()
            .map(|snapshot| snapshot.extensions().clone())
            .unwrap_or_default();
        if !extensions
            .iter()
            .any(crate::extension_generation::reads_configuration)
        {
            return Ok(());
        }
        let configs = db
            .read(move |conn| {
                Ok(crate::extension_generation::config_signature_tx(
                    conn,
                    &extensions,
                ))
            })
            .await?;
        if pin.state.configs != configs {
            self.request();
        }
        Ok(())
    }

    /// Initialize before ingress/background extension consumers are exposed.
    /// The baseline is required bundled code only, with no external instances.
    /// A rejected full candidate keeps this baseline available for repair.
    /// First the live set starts (`extension_deploy::start`): an enabled
    /// extension with no deployment here deploys from its source.
    pub async fn initialize(&self, svc: Arc<Services>) -> Result<(), DomainError> {
        let (changes, unstaged) = match crate::extension_deploy::start(&svc).await {
            Ok(started) => started,
            Err(error) => {
                tracing::warn!(%error, "startup deploys failed");
                (LiveChanges::new(), VocabularyErrors::new())
            }
        };
        // A candidate overtaken by a change made while it was staged is
        // staged again: the startup deploys must not be lost with it. An
        // extension that can't activate is held (a bundled one gives way to
        // the copy that ships); a generation that can't publish at all
        // stops the boot.
        let mut attempts = 0;
        loop {
            attempts += 1;
            match self.activate_kind(svc.clone(), &changes).await {
                Ok(_) => break,
                Err(TxError::Aborted(problems))
                    if problems.contains_key("<host>") && attempts < 3 => {}
                Err(error) => {
                    self.record_failure(&svc, error);
                    let why: Vec<String> = self
                        .problems()
                        .into_iter()
                        .flat_map(|(owner, why)| {
                            why.into_iter().map(move |w| format!("{owner}: {w}"))
                        })
                        .collect();
                    return Err(DomainError::Invalid(format!(
                        "the extensions could not activate: {}",
                        why.join("; ")
                    )));
                }
            }
        }
        if !unstaged.is_empty() {
            let mut problems = self.problems.lock();
            for (name, why) in unstaged {
                problems.entry(name).or_default().extend(why);
            }
            drop(problems);
            svc.extension_catalog.runtime_changed();
        }
        Ok(())
    }

    fn record_failure(&self, svc: &Services, error: TxError<VocabularyErrors>) {
        *self.problems.lock() = match error {
            TxError::Aborted(problems) => problems,
            TxError::Storage(_) => VocabularyErrors::from([(
                "<host>".into(),
                vec!["extension activation could not commit its database state".into()],
            )]),
        };
        svc.extension_catalog.runtime_changed();
        tracing::warn!("extension candidate rejected; retaining the active generation");
    }

    /// One serialized owner responds to file/config/entity changes. Consumers
    /// enqueue work; they never wait for publication while retaining a reader.
    pub fn spawn(self: &Arc<Self>, svc: &Arc<Services>) {
        if self.spawned.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let owner = self.clone();
        let services = Arc::downgrade(svc);
        let mut changes = svc.extension_catalog.changes();
        // File watchers start after initialization. Catch a source edit that
        // rejected an in-flight candidate before this receiver existed.
        if svc
            .extension_catalog
            .live_snapshot(&svc.layout.project_dir)
            .source_generation()
            != svc
                .extension_catalog
                .snapshot(&svc.layout.project_dir)
                .source_generation()
        {
            self.request();
        }
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = owner.requested.notified() => {},
                    changed = changes.recv() => match changed {
                        Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                while changes.try_recv().is_ok() {}
                let Some(svc) = services.upgrade() else {
                    return;
                };
                match owner.activate(svc.clone()).await {
                    Ok(_) => {
                        svc.extension_catalog.runtime_changed();
                        let _ = svc
                            .sql
                            .read_generation(svc.metrics.capture_entity_states(true))
                            .await;
                        crate::capabilities::refresh_agent_text(&svc).await;
                    }
                    Err(error) => owner.record_failure(&svc, error),
                }
            }
        });
    }

    /// `spawn_blocking` that keeps the caller's reader scope: a task-local
    /// does not cross into the blocking pool, and the code there reads the
    /// same generation the caller pinned.
    pub fn spawn_blocking<T: Send + 'static>(
        work: impl FnOnce() -> T + Send + 'static,
    ) -> tokio::task::JoinHandle<T> {
        let pins = READERS.try_with(Clone::clone).unwrap_or_default();
        tokio::task::spawn_blocking(move || READERS.sync_scope(pins, work))
    }

    #[cfg(test)]
    pub(crate) fn reader_in_scope() -> bool {
        READERS.try_with(|pins| !pins.is_empty()).unwrap_or(false)
    }

    fn inherited(&self) -> Option<GenerationPin> {
        READERS
            .try_with(|pins| {
                pins.iter()
                    .find(|pin| Arc::ptr_eq(&pin.owner, &self.state))
                    .cloned()
            })
            .ok()
            .flatten()
    }

    /// The generation already retained by this task's reader scope.
    /// This does not acquire a reader or authorize an operation.
    pub fn current_pin(&self) -> Option<GenerationPin> {
        self.inherited()
    }

    pub async fn pin(&self) -> Result<GenerationPin, DomainError> {
        self.pin_as(Cow::Borrowed("ExtensionActivation::pin")).await
    }

    /// This task's pin, or a new one held by `held_by`.
    async fn pin_as(&self, held_by: Cow<'static, str>) -> Result<GenerationPin, DomainError> {
        if let Some(pin) = self.inherited() {
            return Ok(pin);
        }
        let state = self.state.clone().read_owned().await;
        if state.unavailable {
            return Err(DomainError::Invalid(
                "extension activation is unavailable".into(),
            ));
        }
        let id = self.readers.next.fetch_add(1, Ordering::Relaxed);
        self.readers
            .held
            .lock()
            .insert(id, (held_by, Instant::now()));
        Ok(GenerationPin {
            owner: self.state.clone(),
            state: Arc::new(state),
            _held: Arc::new(Held {
                readers: self.readers.clone(),
                id,
            }),
        })
    }

    /// Nested calls inherit their existing pin, including when a writer is
    /// waiting. Reacquiring Tokio's fair lock there would deadlock the parent.
    ///
    /// The reads box `future` before anything else: they wrap whole command,
    /// AI and hook runs, which nest one another, and a debug build's
    /// generic wrappers each kept a copy of the future they wrap, so the
    /// nest outgrew a default thread stack. Boxed once here, every layer
    /// below holds a pointer.
    pub fn read<'a, F>(
        &'a self,
        future: F,
    ) -> impl Future<Output = Result<F::Output, DomainError>> + 'a
    where
        F: Future + 'a,
    {
        let future = Box::pin(future);
        async move {
            let pin = self
                .pin_as(Cow::Borrowed(std::any::type_name::<F>()))
                .await?;
            self.read_pinned(&pin, future).await
        }
    }

    pub fn read_pinned<'a, F>(
        &'a self,
        pin: &'a GenerationPin,
        future: F,
    ) -> impl Future<Output = Result<F::Output, DomainError>> + 'a
    where
        F: Future + 'a,
    {
        let future = Box::pin(future);
        async move {
            if !Arc::ptr_eq(&pin.owner, &self.state) {
                return Err(DomainError::Invalid(
                    "generation pin belongs to another project".into(),
                ));
            }
            let mut pins = READERS.try_with(Clone::clone).unwrap_or_default();
            if !pins
                .iter()
                .any(|existing| Arc::ptr_eq(&existing.owner, &self.state))
            {
                pins.push(pin.clone());
            }
            Ok(READERS.scope(pins, future).await)
        }
    }

    /// A retained context can carry the same pin across a spawned host task.
    /// A typed downcast and boundary identity check prevent accepting a pin
    /// from another project. Fresh job contexts must acquire a fresh pin.
    pub async fn pin_run(&self, run: &RunContext) -> Result<RunContext, DomainError> {
        let pin = match run.host_generation::<GenerationPin>() {
            Some(pin) if Arc::ptr_eq(&pin.owner, &self.state) => pin.clone(),
            Some(_) => {
                return Err(DomainError::Invalid(
                    "generation pin belongs to another project".into(),
                ))
            }
            None => {
                let held_by = match run.contribution_ref() {
                    Some(contribution) => format!("{} (run {})", contribution.id, run.run()),
                    None => format!("{} (run {})", run.actor().source(), run.run()),
                };
                self.pin_as(Cow::Owned(held_by)).await?
            }
        };
        Ok(run.clone().with_host_generation(Arc::new(pin)))
    }

    /// Stage one candidate, then reserve the command/capability/provider maps,
    /// commit matching SQL and publish the maps while all readers are excluded.
    /// A preparation/SQL rejection leaves the previous pin available. A panic
    /// after SQL commit leaves this boundary unavailable until host recovery.
    /// Provider startup and retired process shutdown happen outside SQL
    /// transactions. Boot and subsequent catalog changes use this path.
    pub async fn activate(
        &self,
        svc: Arc<Services>,
    ) -> Result<Arc<CatalogSnapshot>, TxError<VocabularyErrors>> {
        self.activate_kind(svc, &LiveChanges::new()).await
    }

    /// Activate with `changes` to the live set, holding what doesn't
    /// prepare as any activation does (startup, and tests that stage
    /// sources).
    pub async fn activate_live(
        &self,
        svc: Arc<Services>,
        changes: &LiveChanges,
    ) -> Result<Arc<CatalogSnapshot>, TxError<VocabularyErrors>> {
        self.activate_kind(svc, changes).await
    }

    /// Activate with `changes` to the live set (a deploy, a rollback, a
    /// removal). The publication records which deployments are active. An
    /// extension `changes` names that doesn't go live is held as any broken
    /// one is (its last working version stays) and its problems are the
    /// error.
    pub async fn activate_changes(
        &self,
        svc: Arc<Services>,
        changes: &LiveChanges,
    ) -> Result<Arc<CatalogSnapshot>, TxError<VocabularyErrors>> {
        let published = self.activate_kind(svc, changes).await?;
        let held = self.problems.lock().clone();
        let failed: VocabularyErrors = changes
            .keys()
            .filter_map(|name| held.get(name).map(|why| (name.clone(), why.clone())))
            .collect();
        if failed.is_empty() {
            Ok(published)
        } else {
            Err(TxError::Aborted(failed))
        }
    }

    /// Restate the listing rows that follow a source on disk (an
    /// undeployed extension, one named but not here) from the sources as
    /// they are now, without an activation: nothing that runs changes. The
    /// file watcher calls it when the extensions folder changes, so a
    /// deleted or added source shows at once. It waits for an activation in
    /// progress, so a publication's listing is never overwritten by an
    /// older one.
    pub async fn restate_sources(
        &self,
        svc: Arc<Services>,
    ) -> Result<(), oxplow_domain::DomainError> {
        let _one = self.activating.lock().await;
        let snapshot = svc.extension_catalog.snapshot(&svc.layout.project_dir);
        let rows: Vec<_> = snapshot
            .extensions()
            .iter()
            .map(crate::extension_generation::extension_row)
            .collect();
        svc.db
            .transaction(move |tx| {
                oxplow_db::catalog_listing_store::restate_source_backed_tx(tx, &rows)
            })
            .await
    }

    async fn activate_kind(
        &self,
        svc: Arc<Services>,
        changes: &LiveChanges,
    ) -> Result<Arc<CatalogSnapshot>, TxError<VocabularyErrors>> {
        if !std::ptr::eq(self, svc.extension_activation.as_ref()) {
            return Err(rejected(
                "activation owner belongs to another project boundary",
            ));
        }
        if self.inherited().is_some() {
            return Err(rejected("activation cannot wait on its own reader"));
        }
        let _one = self.activating.lock().await;
        let _staged = svc.extension_catalog.stage_changes(changes);
        let settings = svc.extension_catalog.settings(&svc.layout.project_dir);
        let mut raw = svc.extension_catalog.snapshot(&svc.layout.project_dir);
        // An extension that can't load keeps its last working version (or
        // stays out); the rest activate without it. One whose preparation
        // fails (a model that doesn't compile, a provider that can't stage)
        // is held the same way and the candidate prepared again.
        let mut failed = crate::extension_catalog::Failed::new();
        let mut gave_way = VocabularyErrors::new();
        let mut restaged = false;
        let (published, retired, mut held) = loop {
            let (snapshot, held) = raw.admitted(svc.extension_catalog.active().as_deref(), &failed);
            // A deployment of an extension that ships with oxplow that is
            // held with nothing of it running gives way to the copy that
            // ships, and the candidate is admitted again.
            let giving_way: Vec<(String, Vec<String>)> = held
                .iter()
                .filter(|(name, _)| {
                    !snapshot
                        .extensions()
                        .iter()
                        .any(|e| &e.name == *name && e.enabled)
                        && raw.extensions().iter().any(|e| {
                            &e.name == *name
                                && matches!(
                                    e.deployment,
                                    crate::extensions::Deployment::Deployed { .. }
                                )
                        })
                        && crate::bundled_extensions::shipped(name, &settings).is_some()
                })
                .map(|(name, why)| (name.clone(), why.clone()))
                .collect();
            if !giving_way.is_empty() {
                for (name, mut why) in giving_way {
                    tracing::warn!(%name, "a deployment of an extension that ships with oxplow doesn't activate; running the shipped one");
                    svc.extension_catalog.give_way(&name);
                    failed.forget(&name);
                    why.retain(|w| !w.ends_with("until these are fixed"));
                    why.push(
                        "its deployment doesn't activate; the shipped version runs until it is deployed again"
                            .into(),
                    );
                    gave_way.insert(name, why);
                }
                raw = svc.extension_catalog.snapshot(&svc.layout.project_dir);
                continue;
            }
            let prepared = match Self::prepare(&svc, &snapshot).await {
                Ok(prepared) => prepared,
                Err(TxError::Aborted(problems)) if failed.hold(&snapshot, &problems) => continue,
                Err(error) => return Err(error),
            };
            let Prepared {
                commands,
                adapters,
                bundles,
                providers,
                metrics,
                capabilities,
                mut database,
            } = prepared;
            database.list_held(&held, &raw);
            let db = svc.db.clone();
            let published = snapshot.clone();
            // Move the owned writer into the worker closure. Cancelling the async
            // caller must not open readers while that worker can still commit.
            let boundary = Mutex::new(self.write_waiting_on_readers().await);
            // Provider admission may hold this map while awaiting a DB write. Acquire
            // it before the DB permit, otherwise the publication worker can deadlock.
            let running = Mutex::new(svc.providers.reserve_running().await);
            let providers = Mutex::new(providers);
            let adapters = Mutex::new(Some(adapters));
            let bundles = Mutex::new(Some(bundles));
            let svc = svc.clone();
            let outcome = db
                .transaction_with_publication(move |publication| {
                    let mut boundary = boundary.lock().unwrap_or_else(|error| error.into_inner());
                    let mut commands = svc.extension_commands.reserve(&commands).map_err(|_| {
                        rejected("command registry changed while activation was staged")
                    })?;
                    // A snapshot, not a held read guard: a provider's live check
                    // below reads the configuration again, and a second read
                    // behind a waiting writer (`reload_config_from_disk`) never
                    // returns. A change after this is a new activation's.
                    let config = crate::config_service::read_config(&svc.config);
                    let mut running = running.lock().unwrap_or_else(|error| error.into_inner());
                    let mut providers = providers.lock().unwrap_or_else(|error| error.into_inner());
                    let mut providers = svc
                        .providers
                        .reserve_generation(&mut providers, &mut running, &config)
                        .map_err(TxError::Aborted)?;
                    let knowledge = capabilities
                        .selected("knowledge")
                        .unwrap_or(oxplow_domain::capability::NONE)
                        .to_string();
                    let snapshots = capabilities
                        .selected("snapshots")
                        .unwrap_or(oxplow_domain::capability::NONE)
                        .to_string();
                    let mut capabilities = svc
                        .capabilities
                        .reserve_prepared(&capabilities, &config)
                        .map_err(|_| {
                            rejected(
                                "capability registry or choices changed while activation was staged",
                            )
                        })?;
                    if svc
                        .extension_catalog
                        .snapshot(&svc.layout.project_dir)
                        .generation()
                        != published.source_generation()
                    {
                        return Err(rejected("catalog changed while activation was staged"));
                    }
                    let live: Vec<(String, String)> =
                        crate::extension_catalog::live_of(published.extensions())
                            .into_iter()
                            .collect();
                    oxplow_db::extension_deployment_store::set_active_tx(
                        publication.transaction()?,
                        &live,
                    )
                    .map_err(TxError::Storage)?;
                    // An effect starts when a generation that runs it
                    // first publishes: it never reacts to the past.
                    let now = Timestamp::now().to_string();
                    for ext in published.extensions().iter().filter(|e| e.enabled) {
                        for effect in &ext.effects {
                            oxplow_db::effect_state_store::start_if_new_tx(
                                publication.transaction()?,
                                &effect.name(),
                                &now,
                            )
                            .map_err(TxError::Storage)?;
                        }
                    }
                    // Stored settings behind an extension's declared schema
                    // are carried forward in this transaction: a step that
                    // fails rolls everything back and holds that extension.
                    crate::extension_config_migration::migrate_all_tx(
                        publication.transaction()?,
                        published.extensions().iter(),
                    )
                    .map_err(TxError::Aborted)?;
                    let vocabulary =
                        database.replace_tx(publication.transaction()?, Timestamp::now())?;
                    metrics.replace_tx(publication.transaction()?)?;
                    providers
                        .validate_vocabulary(&vocabulary)
                        .map_err(TxError::Aborted)?;
                    providers.validate_live().map_err(TxError::Aborted)?;
                    let entities =
                        crate::extension_models::entity_signature_tx(publication.transaction()?)?;
                    let configs = crate::extension_generation::config_signature_tx(
                        publication.transaction()?,
                        published.extensions(),
                    );
                    // Set this before the commit attempt so an unwind in either commit
                    // or publication cannot expose mismatched database/memory state.
                    boundary.unavailable = true;
                    if let Err(error) = publication.commit() {
                        boundary.unavailable = false;
                        return Err(TxError::Storage(error));
                    }
                    #[cfg(test)]
                    svc.extension_activation
                        .maybe_panic(PanicPoint::AfterCommit);
                    commands.apply();
                    capabilities.apply();
                    svc.vocabulary.swap(vocabulary);
                    metrics.publish(&svc.metrics);
                    providers.withdraw_maps();
                    adapters
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take()
                        .expect("adapter candidate consumed only after successful commit")
                        .publish_maps(&svc);
                    svc.feeds.restate(published.extensions());
                    bundles
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take()
                        .expect("bundle candidate consumed only after successful commit")
                        .publish(&svc.component_bundles);
                    svc.extension_catalog.publish_active(published.clone());
                    svc.extension_catalog
                        .set_live(crate::extension_catalog::live_of(published.extensions()));
                    // Admission enables callbacks and their live-owner checks.
                    // Publish the matching owner catalog before starting them;
                    // readers remain excluded by the owned generation writer.
                    let retired = providers.apply();
                    crate::extension_runtime_adapters::PreparedRuntimeAdapters::follow_selected(
                        &svc, &knowledge, &snapshots,
                    );
                    #[cfg(test)]
                    svc.extension_activation
                        .maybe_panic(PanicPoint::BeforeBoundary);
                    boundary.active = Some(published.clone());
                    svc.extension_activation
                        .activated
                        .store(true, std::sync::atomic::Ordering::Release);
                    boundary.entities = entities;
                    boundary.configs = configs;
                    boundary.unavailable = false;
                    Ok((published.clone(), retired))
                })
                .await;
            match outcome {
                Ok((published, retired)) => break (published, retired, held),
                // A model or vocabulary that doesn't compile is checked in
                // the transaction, which rolled back: nothing was applied.
                Err(TxError::Aborted(problems)) if failed.hold(&snapshot, &problems) => {}
                // Publication panicked after its database state committed,
                // leaving the in-memory maps part swapped and the boundary
                // refusing readers. The database is the truth: stage the
                // generation again from it, once.
                Err(TxError::Storage(error)) if self.state.read().await.unavailable => {
                    if restaged {
                        return Err(rejected(&format!(
                            "extension publication failed after its database state \
                             committed, and staging it again failed: {error}"
                        )));
                    }
                    tracing::error!(%error, "extension publication failed after commit; staging it again");
                    restaged = true;
                }
                Err(error) => return Err(error),
            }
        };
        retired.finish().await;
        for (name, why) in gave_way {
            held.entry(name).or_default().extend(why);
        }
        *self.problems.lock() = held;
        Ok(published)
    }
}

impl ExtensionActivation {
    /// The publication's writer, saying who it waits for while readers
    /// hold the generation. The one write future is kept, so its place in
    /// the fair lock's queue is too.
    async fn write_waiting_on_readers(&self) -> OwnedRwLockWriteGuard<GenerationState> {
        let started = Instant::now();
        let write = self.state.clone().write_owned();
        tokio::pin!(write);
        loop {
            tokio::select! {
                guard = &mut write => return guard,
                _ = tokio::time::sleep(WAITING_LOG_EVERY) => {
                    tracing::warn!(
                        waited = ?started.elapsed(),
                        readers = ?self.readers(),
                        "extension activation is waiting for the generation's readers"
                    );
                }
            }
        }
    }
}

struct Prepared {
    commands: crate::extension_commands::StagedExtensionCommands,
    adapters: crate::extension_runtime_adapters::PreparedRuntimeAdapters,
    bundles: crate::component_bundles::PreparedComponentBundles,
    providers: crate::providers::registry::PreparedProviderGeneration,
    metrics: crate::metrics_service::PreparedMetricCatalog,
    capabilities: crate::capabilities::PreparedCapabilities,
    database: crate::extension_generation::PreparedExtensionDatabase,
}

impl ExtensionActivation {
    /// Stage every registry's candidate for `snapshot`, outside SQL.
    async fn prepare(
        svc: &Arc<Services>,
        snapshot: &Arc<CatalogSnapshot>,
    ) -> Result<Prepared, TxError<VocabularyErrors>> {
        let settings = oxplow_config::extension_settings(&svc.layout.project_dir);
        let blocked: VocabularyErrors = snapshot
            .extensions()
            .iter()
            .filter(|extension| {
                extension.public_contracts.blocked
                    && !crate::extensions::is_off(&settings, &extension.name)
            })
            .map(|extension| (extension.name.clone(), extension.errors.clone()))
            .collect();
        if !blocked.is_empty() {
            return Err(TxError::Aborted(blocked));
        }
        let commands = svc
            .extension_commands
            .stage(snapshot)
            .await
            .map_err(|problems| {
                TxError::Aborted(
                    problems
                        .into_iter()
                        .map(|(owner, problem)| (owner, vec![problem]))
                        .collect(),
                )
            })?;
        let adapters =
            crate::extension_runtime_adapters::PreparedRuntimeAdapters::prepare(svc, snapshot)
                .map_err(TxError::Aborted)?;
        let bundle_root = svc.layout.project_dir.clone();
        let bundle_snapshot = snapshot.clone();
        let bundles = tokio::task::spawn_blocking(move || {
            crate::component_bundles::PreparedComponentBundles::prepare(
                &bundle_root,
                &bundle_snapshot,
            )
        })
        .await
        .map_err(|_| rejected("component bundle preparation failed"))?
        .map_err(TxError::Aborted)?;
        let providers = svc
            .providers
            .prepare_generation(snapshot)
            .await
            .map_err(TxError::Aborted)?;
        let config = crate::config_service::read_config(&svc.config);
        let metrics: crate::metrics_service::PreparedMetricCatalog = svc
            .metrics
            .prepare_generation(snapshot, &config)
            .map_err(TxError::Aborted)?;
        let capabilities = svc
            .capabilities
            .prepare_with_external(snapshot, &config, providers.implementations())
            .map_err(TxError::Aborted)?;
        let database =
            crate::extension_generation::PreparedExtensionDatabase::prepare_with_capabilities(
                snapshot,
                &capabilities,
            )
            .map_err(TxError::Aborted)?;
        Ok(Prepared {
            commands,
            adapters,
            bundles,
            providers,
            metrics,
            capabilities,
            database,
        })
    }
}

fn rejected(message: &str) -> TxError<VocabularyErrors> {
    TxError::Aborted(VocabularyErrors::from([(
        "<host>".into(),
        vec![message.into()],
    )]))
}
