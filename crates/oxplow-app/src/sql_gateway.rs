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
}

impl SqlGateway {
    pub fn new(db: Database) -> Self {
        Self {
            layer: SemanticLayer::new(db),
            engine: None,
            watermarks: None,
            overlay: std::sync::Arc::new(Vec::new()),
        }
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
    pub async fn run(&self, mut query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        // Boxed: the engine reads entity metrics through a gateway, so
        // this future contains another `run`.
        let (sql, temp, measures) =
            Box::pin(self.materialize(&query.sql, query.stream, true)).await?;
        query.sql = sql;
        query.temp.extend(temp);
        query.temp_views.extend(self.overlay.iter().cloned());
        let mut out = self.layer.run(query).await?;
        out.reads.measures = measures;
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
        self.layer.recent_events(types, limit).await
    }

    /// The name of every view in the database.
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.layer.view_names().await
    }
}
