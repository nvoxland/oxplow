//! The SQL gateway (P4.1, `.context/semantic-layer.md`): the one way a
//! query reaches the semantic layer — MCP and IPC `query_sql`, lenses,
//! advisories, entity metrics, extension checks and source inputs all go
//! through it. The database crate's `SemanticLayer` owns the mechanics
//! (the single-read check, the recording authorizer, `query_only`, the
//! row cap and timeout); the gateway is where what needs the rest of the
//! app joins them — the metric function (P4.5) and model freshness
//! (P4.6).

use oxplow_db::{Database, Reads, SchemaEntity, SemanticLayer, SqlCell, SqlQuery, SqlQueryResult};
use oxplow_domain::DomainError;

#[derive(Clone)]
pub struct SqlGateway {
    layer: SemanticLayer,
}

impl SqlGateway {
    pub fn new(db: Database) -> Self {
        Self {
            layer: SemanticLayer::new(db),
        }
    }

    /// Run one read-only query; the result says what it read.
    pub async fn run(&self, query: SqlQuery) -> Result<SqlQueryResult, DomainError> {
        self.layer.run(query).await
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
        self.layer.check(sql).await
    }

    /// The name of every view (until models replace the catalog, P4.2).
    pub async fn view_names(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.layer.view_names().await
    }

    /// The documented core views (until models replace the catalog, P4.2).
    pub async fn describe_schema(&self) -> Result<Vec<SchemaEntity>, DomainError> {
        self.layer.describe_schema().await
    }
}
