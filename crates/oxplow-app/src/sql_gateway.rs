//! The SQL gateway (P4.1, `.context/semantic-layer.md`): the one way a
//! query reaches the semantic layer — MCP and IPC `query_sql`, lenses,
//! advisories, entity metrics, extension checks and source inputs all go
//! through it. The database crate's `SemanticLayer` owns the mechanics
//! (the single-read check, the recording authorizer, `query_only`, the
//! row cap and timeout); the gateway is where what needs the rest of the
//! app joins them — the metric function (P4.5) and model freshness
//! (P4.6).

use oxplow_db::{Database, Reads, SemanticLayer, SqlCell, SqlQuery, SqlQueryResult, TempTable};
use oxplow_domain::DomainError;

#[derive(Clone)]
pub struct SqlGateway {
    layer: SemanticLayer,
    /// What `metric_grid()` reads metrics through; `None` for a gateway
    /// that serves the engine itself (its entity metrics).
    engine: Option<std::sync::Arc<crate::metric_engine::MetricEngine>>,
    /// When each model last changed, for a result's `freshness` (P4.6).
    watermarks: Option<std::sync::Arc<crate::models_changed::ModelWatermarks>>,
}

impl SqlGateway {
    pub fn new(db: Database) -> Self {
        Self {
            layer: SemanticLayer::new(db),
            engine: None,
            watermarks: None,
        }
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

    /// Run one read-only query; the result says what it read. A
    /// `metric_grid()` query has its metrics' series read first — before
    /// any connection is taken — and runs against them as a temp table.
    pub async fn run(&self, mut query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        let Some(plan) = crate::metric_grid::plan(&query.sql)? else {
            return Ok(self.fresh(self.layer.run(query).await?));
        };
        // Boxed: the engine reads entity metrics through a gateway, so
        // this future contains another `run`.
        let grid =
            Box::pin(plan.materialize(&query.sql, self.engine()?, query.stream, true)).await?;
        query.sql = grid.sql;
        query.temp.push(grid.table);
        let mut out = self.layer.run(query).await?;
        out.reads.measures = grid.measures;
        Ok(self.fresh(out))
    }

    fn engine(&self) -> Result<&crate::metric_engine::MetricEngine, DomainError> {
        self.engine.as_deref().ok_or_else(|| {
            DomainError::Invalid("metric_grid() isn't available here (no metric engine)".into())
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
        let Some(plan) = crate::metric_grid::plan(sql)? else {
            return self.layer.check(sql).await;
        };
        // The metrics are resolved (and their dimension checked) but not
        // read; the query compiles against an empty grid.
        let grid = Box::pin(plan.materialize(sql, self.engine()?, None, false)).await?;
        let mut q = SqlQuery::new(grid.sql).limit(Some(1));
        q.temp.push(TempTable {
            rows: Vec::new(),
            ..grid.table
        });
        let mut reads = self.layer.check_with(q).await?;
        reads.measures = grid.measures;
        Ok(reads)
    }

    /// The name of every view in the database.
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.layer.view_names().await
    }
}
