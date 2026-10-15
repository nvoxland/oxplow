//! The SQL gateway (P4.1, `.context/semantic-layer.md`): the one way a
//! query reaches the semantic layer — MCP `oxplow_query_sql` and IPC `query_sql`, lenses,
//! advisories, entity metrics, extension checks and source inputs all go
//! through it. The database crate's `SemanticLayer` owns the mechanics
//! (the single-read check, the recording authorizer, `query_only`, the
//! row cap and timeout); the gateway is where what needs the rest of the
//! app joins them — the metric functions (`metric_grid()`, P4.5;
//! `metric_findings()`, P4.8) and model freshness (P4.6).

use oxplow_db::{
    Database, Reads, SemanticLayer, SqlCell, SqlQuery, SqlQueryResult, TempTable, TempView,
};
use oxplow_domain::DomainError;

#[derive(Clone)]
pub struct SqlGateway {
    layer: SemanticLayer,
    /// What `metric_grid()` reads metrics through; `None` for a gateway
    /// that serves the engine itself (its entity metrics).
    engine: Option<std::sync::Arc<crate::metric_engine::MetricEngine>>,
    /// When each model last changed, for a result's `freshness` (P4.6).
    watermarks: Option<std::sync::Arc<crate::models_changed::ModelWatermarks>>,
    /// Temp views every query recreates on its connection: a check's
    /// overlay of what an extension declares but hasn't published (P7.C6).
    overlay: std::sync::Arc<Vec<TempView>>,
    /// What a result's `coverage` and `generation` come from.
    collection: Option<std::sync::Arc<Collection>>,
    /// The project's activation boundary. A gateway built alone has its own,
    /// which nothing has activated: it serves the current load.
    activation: std::sync::Arc<crate::extension_activation::ExtensionActivation>,
    /// The event types the log speaks (the app's; core's unless set):
    /// what a review reads logged events through.
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    /// What the live models (`crate::live_models`) are read from; shared by
    /// every clone, bound once by the app.
    live: std::sync::Arc<crate::live_models::LiveModels>,
}

/// What a query result says about the data behind it: which collectors
/// feed the models it read and how they stand, and the catalog's
/// generation (`crate::collection_coverage`).
pub struct Collection {
    pub jobs: std::sync::Arc<crate::extension_jobs::ExtensionJobs>,
    pub ai: std::sync::Arc<crate::ai_service::AiService>,
    pub feeds: std::sync::Arc<crate::collection_coverage::Feeds>,
    pub db: Database,
    pub catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
    pub root: std::path::PathBuf,
}

impl SqlGateway {
    pub fn new(db: Database) -> Self {
        Self {
            layer: SemanticLayer::new(db),
            engine: None,
            watermarks: None,
            overlay: std::sync::Arc::new(Vec::new()),
            collection: None,
            activation: std::sync::Arc::new(crate::extension_activation::ExtensionActivation::new()),
            vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
            live: std::sync::Arc::new(crate::live_models::LiveModels::default()),
        }
    }

    /// The handle the app binds the live models' sources through.
    pub fn live_models(&self) -> &crate::live_models::LiveModels {
        &self.live
    }

    /// A gateway over `db` that reads metrics too (`metric_grid()`,
    /// `metric_findings()`) from the facts in `db`: what a check dry-runs a
    /// metric's lens on, outside a running oxplow.
    pub fn with_metrics(db: Database) -> Self {
        let engine =
            crate::metric_engine::MetricEngine::new(oxplow_db::SqliteFactStore::new(db.clone()));
        Self::new(db).with_engine(engine)
    }

    /// Also resolve the metrics `specs` declare (an extension under check)
    /// that this gateway's database has not stored yet.
    pub fn with_declared_metrics(&self, specs: Vec<oxplow_db::MetricSpec>) -> Self {
        let mut out = self.clone();
        out.engine = self
            .engine
            .as_ref()
            .map(|engine| std::sync::Arc::new(engine.as_ref().clone().with_declared(specs)));
        out
    }

    /// Speak `vocabulary` (the app's).
    pub fn with_vocabulary(
        mut self,
        vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    ) -> Self {
        self.vocabulary = vocabulary;
        self
    }

    /// Say how current the collected data behind a query is, and which
    /// catalog generation it ran against.
    pub fn with_collection(mut self, collection: Collection) -> Self {
        self.collection = Some(std::sync::Arc::new(collection));
        self
    }

    /// Pin queries to the project's extension generation: the boundary the
    /// project's other services share. Set before the gateway is cloned.
    pub fn with_activation(
        mut self,
        activation: std::sync::Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Self {
        self.activation = activation;
        self
    }

    pub(crate) fn request_activation(&self) {
        self.activation.request();
    }

    /// Re-enter a retained run's reader scope before looking up declarations.
    pub(crate) async fn read_in<F: std::future::Future>(
        &self,
        run: &oxplow_domain::run::RunContext,
        future: F,
    ) -> Result<F::Output, DomainError> {
        let run = self.activation.pin_run(run).await?;
        let pin = run
            .host_generation::<crate::extension_activation::GenerationPin>()
            .expect("pinned gateway context");
        self.activation.read_pinned(pin, future).await
    }

    /// Retain this gateway's generation before looking up declarations or
    /// handlers which a later query/invocation will use. Standalone gateways
    /// execute directly; nested scopes inherit their current pin.
    pub(crate) async fn read_generation<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, DomainError> {
        self.activation.read(future).await
    }

    pub(crate) async fn pin_run(
        &self,
        run: &oxplow_domain::run::RunContext,
    ) -> Result<oxplow_domain::run::RunContext, DomainError> {
        self.activation.pin_run(run).await
    }

    /// Spawned/script worker calls must re-enter using the retained host pin;
    /// task-local inheritance alone does not cross those task/thread boundaries.
    pub(crate) async fn run_in(
        &self,
        run: &oxplow_domain::run::RunContext,
        query: SqlQuery,
    ) -> Result<SqlQueryResult, DomainError> {
        let run = self.activation.pin_run(run).await?;
        let pin = run
            .host_generation::<crate::extension_activation::GenerationPin>()
            .expect("pinned gateway context");
        self.activation
            .read_pinned(pin, self.run_unpinned(query))
            .await?
    }

    /// A separate private state reader for script hosts sharing this project's
    /// resources. State never enters `run`, overlays, models or SQL results.
    pub(crate) fn state_source(&self) -> Option<crate::extension_state::StateSource> {
        self.collection.as_ref().map(|resources| {
            crate::extension_state::StateSource::new(
                crate::extension_state::StateHost::new(
                    resources.catalog.clone(),
                    resources.root.clone(),
                ),
                resources.db.clone(),
            )
        })
    }

    pub(crate) fn jobs_source(
        &self,
    ) -> Option<std::sync::Arc<crate::extension_jobs::ExtensionJobs>> {
        self.collection
            .as_ref()
            .map(|resources| resources.jobs.clone())
    }

    pub(crate) fn ai_source(&self) -> Option<std::sync::Arc<crate::ai_service::AiService>> {
        self.collection
            .as_ref()
            .map(|resources| resources.ai.clone())
    }

    pub(crate) fn configuration_source(
        &self,
    ) -> Option<crate::extension_configuration::ConfigurationSource> {
        self.collection.as_ref().map(|resources| {
            crate::extension_configuration::ConfigurationSource::new(
                crate::extension_configuration::ConfigurationHost::new(
                    resources.catalog.clone(),
                    resources.root.clone(),
                ),
                resources.db.clone(),
            )
        })
    }

    /// This gateway with `views` created (in order) for every query it
    /// runs or checks — what a check reads an extension's unpublished
    /// models and unsynced entities through.
    pub fn with_overlay(&self, views: Vec<TempView>) -> Self {
        let mut out = self.clone();
        out.overlay = std::sync::Arc::new(views);
        out
    }

    /// Say how fresh each read model is (P4.6).
    pub fn with_watermarks(
        mut self,
        watermarks: std::sync::Arc<crate::models_changed::ModelWatermarks>,
    ) -> Self {
        self.watermarks = Some(watermarks);
        self
    }

    fn fresh(&self, mut out: SqlQueryResult) -> SqlQueryResult {
        if let Some(w) = &self.watermarks {
            out.freshness = w.freshness(&out.reads.models);
        }
        out
    }

    /// Read metrics in SQL through `engine` (`metric_grid()`, P4.5).
    pub fn with_engine(mut self, engine: crate::metric_engine::MetricEngine) -> Self {
        self.engine = Some(std::sync::Arc::new(engine));
        self
    }

    /// Run one read-only query; the result says what it read. A query
    /// reading metrics — `metric_grid()`, `metric_findings()` — has them
    /// computed first, before any connection is taken, and runs against
    /// them as temp tables.
    pub async fn run(&self, query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        self.activation.read(self.run_unpinned(query)).await?
    }

    async fn run_unpinned(&self, mut query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        if query.model_policy.is_some()
            && (crate::metric_grid::plan(&query.sql)?.is_some()
                || crate::metric_findings::plan(&query.sql)?.is_some())
        {
            return Err(DomainError::Invalid(
                "bounded model grants cannot use metric_grid()/metric_findings(); query granted published models instead".into()
            ));
        }
        // Boxed: the engine reads entity metrics through a gateway, so
        // this future contains another `run`.
        let (sql, temp, views, measures) =
            Box::pin(self.materialize(&query.sql, query.stream, true)).await?;
        query.sql = sql;
        query.temp.extend(temp);
        query.temp_views.extend(self.overlay.iter().cloned());
        query.temp_views.extend(views);
        let mut out = self.layer.run(query).await?;
        out.reads.measures = measures;
        if let Some(c) = &self.collection {
            out.coverage = c.feeds.coverage(&c.db, &out.reads.models).await?;
            out.generation = Some(
                self.activation
                    .current_pin()
                    .and_then(|pin| pin.generation().map(str::to_string))
                    .unwrap_or_else(|| c.catalog.generation(&c.root)),
            );
        } else if let Some(generation) = self
            .activation
            .current_pin()
            .and_then(|pin| pin.generation().map(str::to_string))
        {
            out.generation = Some(generation);
        }
        Ok(self.fresh(out))
    }

    /// `sql` with its metric functions computed (`with_rows`) or only
    /// resolved (a check): the rewritten SQL, its temp tables, and the
    /// measures behind them. Each function is planned over the SQL the
    /// previous one rewrote, so their offsets hold.
    async fn materialize(
        &self,
        sql: &str,
        stream: Option<i64>,
        with_rows: bool,
    ) -> Result<(String, Vec<TempTable>, Vec<TempView>, Vec<String>), DomainError> {
        let mut sql = sql.to_string();
        let mut temp = Vec::new();
        let mut measures = Vec::new();
        // The live models it names are read now, before a connection is
        // taken. A run with nothing bound to read them is refused rather
        // than answered from the empty stub; a check only compiles.
        let live = crate::live_models::LiveModels::named_in(&sql)?;
        if with_rows && !live.is_empty() && !self.live.is_bound() {
            return Err(DomainError::Invalid(format!(
                "v_{} is read live, and this gateway has nothing to read it from",
                live[0]
            )));
        }
        let (live_tables, live_views) = self.live.materialize(&live, with_rows).await?;
        temp.extend(live_tables);
        if let Some(plan) = crate::metric_grid::plan(&sql)? {
            let grid = plan
                .materialize(&sql, self.engine()?, stream, with_rows)
                .await?;
            sql = grid.sql;
            temp.push(grid.table);
            measures.extend(grid.measures);
        }
        if let Some(plan) = crate::metric_findings::plan(&sql)? {
            let found = plan.materialize(&sql, self.engine()?, with_rows).await?;
            sql = found.sql;
            temp.push(found.table);
            measures.extend(found.measures);
        }
        measures.sort();
        measures.dedup();
        Ok((sql, temp, live_views, measures))
    }

    fn engine(&self) -> Result<&crate::metric_engine::MetricEngine, DomainError> {
        self.engine.as_deref().ok_or_else(|| {
            DomainError::Invalid(
                "metric_grid() and metric_findings() aren't available here (no metric engine)"
                    .into(),
            )
        })
    }

    /// [`Self::run`] with positional parameters — the short form.
    pub async fn query_sql(
        &self,
        sql: &str,
        params: Vec<SqlCell>,
        limit: Option<usize>,
    ) -> Result<SqlQueryResult, DomainError> {
        self.run(SqlQuery::new(sql).positional(params).limit(limit))
            .await
    }

    /// Check a query compiles as a read, without running it; say what it
    /// would read.
    pub async fn check(&self, sql: &str) -> Result<Reads, DomainError> {
        self.activation.read(self.check_unpinned(sql)).await?
    }

    async fn check_unpinned(&self, sql: &str) -> Result<Reads, DomainError> {
        // The metrics are resolved (a dimension checked) but not read; the
        // query compiles against empty tables.
        let (rewritten, temp, views, measures) =
            Box::pin(self.materialize(sql, None, false)).await?;
        let mut q = SqlQuery::new(rewritten).limit(Some(1));
        q.temp = temp;
        q.temp_views = self.overlay.to_vec();
        q.temp_views.extend(views);
        let mut reads = self.layer.check_with(q).await?;
        reads.measures = measures;
        Ok(reads)
    }

    /// Check extensions' models compile, publishing nothing (P4.9); `stubs`
    /// stand in for declared entities that haven't synced (P7.C6).
    pub async fn check_extension_models(
        &self,
        extensions: Vec<oxplow_db::models::ExtensionModels>,
        stubs: Vec<oxplow_db::models::EntityStub>,
    ) -> Result<oxplow_db::models::CheckedModels, DomainError> {
        self.layer.check_extension_models(extensions, stubs).await
    }

    /// The latest `limit` events of any of `types`, newest first.
    pub async fn recent_events(
        &self,
        types: Vec<String>,
        limit: usize,
    ) -> Result<Vec<oxplow_domain::StoredEvent>, DomainError> {
        self.activation
            .read(self.recent_events_unpinned(types, limit))
            .await?
    }

    async fn recent_events_unpinned(
        &self,
        types: Vec<String>,
        limit: usize,
    ) -> Result<Vec<oxplow_domain::StoredEvent>, DomainError> {
        self.layer.recent_events(types, limit).await
    }

    /// The event types the log speaks now.
    pub fn vocabulary(&self) -> std::sync::Arc<oxplow_domain::vocabulary::Vocabulary> {
        self.vocabulary.current()
    }

    /// `sql`, an extension's own (a lens's or an advisory's), with each
    /// `config('/pointer')` replaced by the value in that extension's
    /// effective configuration as stored now. Nothing is read when `sql` has
    /// no such call. A change of settings applies to the next run.
    pub(crate) async fn with_extension_config(
        &self,
        ext: &crate::extensions::Extension,
        sql: &str,
    ) -> Result<String, DomainError> {
        if oxplow_db::models::config_reads(sql)?.is_empty() {
            return Ok(sql.to_string());
        }
        let values = match ext.configuration.clone() {
            Some(declaration) => {
                let name = ext.name.clone();
                Some(
                    self.layer
                        .database()
                        .read(move |conn| {
                            crate::extension_configuration::effective_values(
                                conn,
                                &name,
                                &declaration,
                            )
                            .map_err(|error| {
                                DomainError::Invalid(format!(
                                    "the extension's stored configuration can't be read: {error:?}"
                                ))
                            })
                        })
                        .await?,
                )
            }
            None => None,
        };
        oxplow_db::models::substitute_config(sql, values.as_ref())
    }

    /// The name of every view in the database.
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.activation.read(self.view_names_unpinned()).await?
    }

    async fn view_names_unpinned(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.layer.view_names().await
    }
}
