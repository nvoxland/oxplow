//! The SQL gateway (P4.1, `.context/semantic-layer.md`): the one way a
//! query reaches the semantic layer — MCP and IPC `query_sql`, lenses,
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
    /// Optional project activation boundary. Standalone gateways (including
    /// fixtures and candidate checks) retain their existing behavior.
    activation: std::sync::Arc<
        std::sync::OnceLock<std::sync::Arc<crate::extension_activation::ExtensionActivation>>,
    >,
    /// The event types the log speaks (the app's; core's unless set):
    /// what a review reads logged events through.
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

/// What a query result says about the data behind it: which collectors
/// feed the models it read and how they stand, and the catalog's
/// generation (`crate::collection_coverage`).
pub struct Collection {
    pub jobs: std::sync::Arc<crate::extension_jobs::ExtensionJobs>,
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
            activation: std::sync::Arc::new(std::sync::OnceLock::new()),
            vocabulary: oxplow_domain::vocabulary::VocabularyHandle::core(),
        }
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

    /// Pin queries to the project's active extension generation.
    pub fn with_activation(
        self,
        activation: std::sync::Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Result<Self, DomainError> {
        self.adopt_activation(activation)?;
        Ok(self)
    }

    /// Adopt the project's shared activation boundary. Gateways cloned before
    /// boot (including metric gateways) observe the same adoption.
    pub(crate) fn adopt_activation(
        &self,
        activation: std::sync::Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Result<(), DomainError> {
        let adopted = self.activation.get_or_init(|| activation.clone());
        if std::sync::Arc::ptr_eq(adopted, &activation) {
            Ok(())
        } else {
            Err(DomainError::Invalid(
                "SQL gateway already belongs to another extension activation".into(),
            ))
        }
    }

    pub(crate) fn has_activation(&self) -> bool {
        self.activation.get().is_some()
    }

    pub(crate) fn request_activation(&self) {
        if let Some(activation) = self.activation.get() {
            activation.request();
        }
    }

    /// Re-enter a retained run's reader scope before looking up declarations.
    pub(crate) async fn read_in<F: std::future::Future>(
        &self,
        run: &oxplow_domain::run::RunContext,
        future: F,
    ) -> Result<F::Output, DomainError> {
        match self.activation.get() {
            Some(activation) => {
                let run = activation.pin_run(run).await?;
                let pin = run
                    .host_generation::<crate::extension_activation::GenerationPin>()
                    .expect("pinned gateway context");
                activation.read_pinned(pin, future).await
            }
            None => Ok(future.await),
        }
    }

    /// Retain this gateway's generation before looking up declarations or
    /// handlers which a later query/invocation will use. Standalone gateways
    /// execute directly; nested scopes inherit their current pin.
    pub(crate) async fn read_generation<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, DomainError> {
        match self.activation.get() {
            Some(activation) => activation.read(future).await,
            None => Ok(future.await),
        }
    }

    pub(crate) async fn pin_run(
        &self,
        run: &oxplow_domain::run::RunContext,
    ) -> Result<oxplow_domain::run::RunContext, DomainError> {
        match self.activation.get() {
            Some(activation) => activation.pin_run(run).await,
            None => Ok(run.clone()),
        }
    }

    /// Spawned/script worker calls must re-enter using the retained host pin;
    /// task-local inheritance alone does not cross those task/thread boundaries.
    pub(crate) async fn run_in(
        &self,
        run: &oxplow_domain::run::RunContext,
        query: SqlQuery,
    ) -> Result<SqlQueryResult, DomainError> {
        if let Some(activation) = self.activation.get() {
            let run = activation.pin_run(run).await?;
            let pin = run
                .host_generation::<crate::extension_activation::GenerationPin>()
                .expect("pinned gateway context");
            return activation
                .read_pinned(pin, self.run_unpinned(query))
                .await?;
        }
        self.run_unpinned(query).await
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
        if let Some(activation) = self.activation.get() {
            return activation.read(self.run_unpinned(query)).await?;
        }
        self.run_unpinned(query).await
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
        let (sql, temp, measures) =
            Box::pin(self.materialize(&query.sql, query.stream, true)).await?;
        query.sql = sql;
        query.temp.extend(temp);
        query.temp_views.extend(self.overlay.iter().cloned());
        let mut out = self.layer.run(query).await?;
        out.reads.measures = measures;
        if let Some(c) = &self.collection {
            out.coverage = c.feeds.coverage(&c.db, &out.reads.models).await?;
            out.generation = Some(
                self.activation
                    .get()
                    .and_then(|activation| activation.current_pin())
                    .map(|pin| pin.generation().to_string())
                    .unwrap_or_else(|| c.catalog.generation(&c.root)),
            );
        } else if let Some(generation) = self
            .activation
            .get()
            .and_then(|activation| activation.current_pin())
            .map(|pin| pin.generation().to_string())
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
    ) -> Result<(String, Vec<TempTable>, Vec<String>), DomainError> {
        let mut sql = sql.to_string();
        let mut temp = Vec::new();
        let mut measures = Vec::new();
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
        Ok((sql, temp, measures))
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
        if let Some(activation) = self.activation.get() {
            return activation.read(self.check_unpinned(sql)).await?;
        }
        self.check_unpinned(sql).await
    }

    async fn check_unpinned(&self, sql: &str) -> Result<Reads, DomainError> {
        // The metrics are resolved (a dimension checked) but not read; the
        // query compiles against empty tables.
        let (rewritten, temp, measures) = Box::pin(self.materialize(sql, None, false)).await?;
        let mut q = SqlQuery::new(rewritten).limit(Some(1));
        q.temp = temp;
        q.temp_views = self.overlay.to_vec();
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
        if let Some(activation) = self.activation.get() {
            return activation
                .read(self.recent_events_unpinned(types, limit))
                .await?;
        }
        self.recent_events_unpinned(types, limit).await
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

    /// The name of every view in the database.
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        if let Some(activation) = self.activation.get() {
            return activation.read(self.view_names_unpinned()).await?;
        }
        self.view_names_unpinned().await
    }

    async fn view_names_unpinned(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.layer.view_names().await
    }
}
