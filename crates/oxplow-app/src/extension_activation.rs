//! Publication and reader boundary for prepared extension generations.
//! Boot adopts this boundary before exposing extension runtime consumers.

use std::future::Future;
use std::sync::{Arc, Mutex};

use oxplow_db::TxError;
use oxplow_domain::{run::RunContext, DomainError, Timestamp};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};

use crate::extension_catalog::CatalogSnapshot;
use crate::vocabulary_reactor::VocabularyErrors;
use crate::Services;

#[derive(Default)]
struct GenerationState {
    active: Option<Arc<CatalogSnapshot>>,
    unavailable: bool,
    entities: crate::extension_models::EntitySignature,
}

/// A host-only reader lifetime. Its private guard excludes publication until
/// the last nested run/query releases it. It conveys no service authority.
#[derive(Clone)]
pub struct GenerationPin {
    owner: Arc<RwLock<GenerationState>>,
    state: Arc<OwnedRwLockReadGuard<GenerationState>>,
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

    /// Initialize before ingress/background extension consumers are exposed.
    /// The baseline is required bundled code only, with no external instances.
    /// A rejected full candidate keeps this baseline available for repair.
    pub async fn initialize(&self, svc: Arc<Services>) -> Result<(), DomainError> {
        if let Err(error) = self.activate_core(svc.clone()).await {
            self.record_failure(&svc, error);
            return Err(DomainError::Invalid(
                "required extension baseline could not activate".into(),
            ));
        }
        if let Err(error) = self.activate(svc.clone()).await {
            self.record_failure(&svc, error);
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
            .runtime_snapshot(&svc.layout.project_dir)
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
        if let Some(pin) = self.inherited() {
            return Ok(pin);
        }
        let state = self.state.clone().read_owned().await;
        if state.unavailable {
            return Err(DomainError::Invalid(
                "extension activation is unavailable".into(),
            ));
        }
        Ok(GenerationPin {
            owner: self.state.clone(),
            state: Arc::new(state),
        })
    }

    /// Nested calls inherit their existing pin, including when a writer is
    /// waiting. Reacquiring Tokio's fair lock there would deadlock the parent.
    pub async fn read<F: Future>(&self, future: F) -> Result<F::Output, DomainError> {
        let pin = self.pin().await?;
        Box::pin(self.read_pinned(&pin, future)).await
    }

    pub async fn read_pinned<F: Future>(
        &self,
        pin: &GenerationPin,
        future: F,
    ) -> Result<F::Output, DomainError> {
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
        // Boxed: a pinned read wraps whole command, AI and hook runs, which
        // nest one another; inline, they overflow a debug thread's stack.
        Ok(Box::pin(READERS.scope(pins, future)).await)
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
            None => self.pin().await?,
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
        self.activate_kind(svc, false).await
    }

    /// Publish only required bundled declarations so the host can repair a
    /// failed first extension candidate without exposing a partial activation.
    pub async fn activate_core(
        &self,
        svc: Arc<Services>,
    ) -> Result<Arc<CatalogSnapshot>, TxError<VocabularyErrors>> {
        self.activate_kind(svc, true).await
    }

    async fn activate_kind(
        &self,
        svc: Arc<Services>,
        core_only: bool,
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
        let raw = svc.extension_catalog.snapshot(&svc.layout.project_dir);
        // An extension that can't load keeps its last working version (or
        // stays out); the rest activate without it. One whose preparation
        // fails (a model that doesn't compile, a provider that can't stage)
        // is held the same way and the candidate prepared again.
        let mut failed = crate::extension_catalog::Failed::new();
        let (published, retired, held) = loop {
            let (snapshot, held) = if core_only {
                (raw.core_baseline(), VocabularyErrors::new())
            } else {
                raw.admitted(svc.extension_catalog.active().as_deref(), &failed)
            };
            let prepared = match Self::prepare(&svc, &snapshot, core_only).await {
                Ok(prepared) => prepared,
                Err(TxError::Aborted(problems))
                    if !core_only && failed.hold(&snapshot, &problems) =>
                {
                    continue
                }
                Err(error) => return Err(error),
            };
            let Prepared {
                commands,
                adapters,
                bundles,
                providers,
                metrics,
                capabilities,
                database,
            } = prepared;
            let db = svc.db.clone();
            let published = snapshot.clone();
            // Move the owned writer into the worker closure. Cancelling the async
            // caller must not open readers while that worker can still commit.
            let boundary = Mutex::new(self.state.clone().write_owned().await);
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
                    let config = svc
                        .config
                        .read()
                        .map_err(|_| rejected("host configuration is unavailable"))?;
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
                    let vocabulary =
                        database.replace_tx(publication.transaction()?, Timestamp::now())?;
                    metrics.replace_tx(publication.transaction()?)?;
                    providers
                        .validate_vocabulary(&vocabulary)
                        .map_err(TxError::Aborted)?;
                    providers.validate_live().map_err(TxError::Aborted)?;
                    let entities =
                        crate::extension_models::entity_signature_tx(publication.transaction()?)?;
                    // Set this before the commit attempt so an unwind in either commit
                    // or publication cannot expose mismatched database/memory state.
                    boundary.unavailable = true;
                    if let Err(error) = publication.commit() {
                        boundary.unavailable = false;
                        return Err(TxError::Storage(error));
                    }
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
                    // Admission enables callbacks and their live-owner checks.
                    // Publish the matching owner catalog before starting them;
                    // readers remain excluded by the owned generation writer.
                    let retired = providers.apply();
                    crate::extension_runtime_adapters::PreparedRuntimeAdapters::follow_selected(
                        &svc, &knowledge, &snapshots,
                    );
                    boundary.active = Some(published.clone());
                    svc.extension_activation
                        .activated
                        .store(true, std::sync::atomic::Ordering::Release);
                    boundary.entities = entities;
                    boundary.unavailable = false;
                    Ok((published.clone(), retired))
                })
                .await;
            match outcome {
                Ok((published, retired)) => break (published, retired, held),
                // A model or vocabulary that doesn't compile is checked in
                // the transaction, which rolled back: nothing was applied.
                Err(TxError::Aborted(problems))
                    if !core_only && failed.hold(&snapshot, &problems) => {}
                Err(error) => return Err(error),
            }
        };
        retired.finish().await;
        if !core_only {
            *self.problems.lock() = held;
        }
        Ok(published)
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
        core_only: bool,
    ) -> Result<Prepared, TxError<VocabularyErrors>> {
        let disabled = oxplow_config::disabled_extensions(&svc.layout.project_dir);
        let blocked: VocabularyErrors = snapshot
            .extensions()
            .iter()
            .filter(|extension| {
                extension.public_contracts.blocked
                    && (crate::bundled_extensions::is_required(&extension.name)
                        || !disabled.contains(&extension.name))
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
        let providers = if core_only {
            svc.providers.prepare_core_generation(snapshot).await
        } else {
            svc.providers.prepare_generation(snapshot).await
        }
        .map_err(TxError::Aborted)?;
        let config = crate::config_service::read_config(&svc.config);
        let metrics: crate::metrics_service::PreparedMetricCatalog = svc
            .metrics
            .prepare_generation(snapshot, &config, core_only)
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
