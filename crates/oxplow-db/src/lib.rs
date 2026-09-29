//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod agent_nudge_store;
pub mod agent_stores;
pub mod ai_call_store;
pub mod analytics_stores;
pub mod attribution_store;
pub mod change_store;
pub mod command_audit_store;
pub mod comment_store;
pub mod dashboard_store;
mod database;
pub mod diagnostic_store;
pub mod effort_evidence_store;
pub mod effort_store;
pub mod event_content_store;
pub mod event_log_store;
pub mod event_retention;
pub mod ext_source_store;
pub mod fact_store;
pub mod git_store;
pub mod models;
pub mod observation_store;
pub mod page_ref_projections;
pub mod page_ref_store;
pub mod reasoning_store;
pub mod search_store;
pub mod semantic_layer;
pub mod snapshot_tree;
pub mod sql_tokens;
mod stream_store;
pub mod task_satellite;
pub mod task_store;
mod thread_store;
pub mod token_usage_store;
pub mod tool_call_store;
pub mod wiki_page_store;
pub mod wiki_page_thread_updates;

pub use agent_nudge_store::{AgentNudge, NewAgentNudge, SqliteAgentNudgeStore};
pub use agent_stores::{SqliteAgentStatusStore, SqliteAgentTurnStore};
pub use ai_call_store::{NewAiCall, SqliteAiCallStore};
pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, Snapshot, SnapshotChangeEntry, SnapshotContentRef, SnapshotOp, SnapshotStats,
    SnapshotStorage, SqliteCodeQualityStore, SqlitePageVisitStore, SqliteSnapshotStore,
    SqliteUsageStore, StampedSnapshot, TakeOutcome, TakeRecord, UsageEvent, UsageRollup,
};
pub use attribution_store::{
    SqliteAttributionStore, STATE_ACKNOWLEDGED, STATE_CLAIMED, STATE_UNATTRIBUTED,
};
pub use change_store::{
    ChangeCoChangeRow, ChangeDuplicateRow, ChangeFileRow, ChangeFunctionRow, ChangeImportRow,
    ChangeResults, ChangeRow, ChangeTestFileRow, SqliteChangeStore,
};
pub use command_audit_store::{CommandAudit, NewCommandAudit, SqliteCommandAuditStore};
pub use comment_store::SqliteCommentStore;
pub use dashboard_store::{
    Dashboard, DashboardItem, DashboardWithItems, NewDashboardItem, SqliteDashboardStore,
};
pub use database::{map_sql_err, Database, DbInitError};
pub use diagnostic_store::{DiagnosticRow, SqliteDiagnosticStore};
pub use effort_evidence_store::SqliteEffortEvidenceStore;
pub use effort_store::{
    Effort, EffortAtSnapshot, EffortChangedPaths, EffortFile, EffortFileChange, EffortStore,
    FileRefVersion, OwnedFileRefVersion, RecordEffortAtomic, SqliteEffortStore,
};
pub use event_log_store::{anchors_for_thread_tx, DeadLetter, EventCtx, SqliteEventLogStore};
pub use ext_source_store::{
    EntityTable, EntityWrite, SourceState, SqliteExtSourceStore, StoredType,
};
pub use fact_store::{
    BatchApply, BatchRows, CubeReadRow, Dimension, EffortMetricDelta, FactRow, FactSliceKey,
    Measure, MetricCapture, MetricSpec, NewCubeRow, NewDimension, NewFact, NewMeasure,
    NewMetricCapture, NewMetricSpec, SqliteFactStore,
};
pub use git_store::{GitBranchRow, GitCommitFileRow, GitCommitRow, SqliteGitStore};
pub use observation_store::EffortObservation;
pub use page_ref_store::{PageRefEdge, PageRefStore, SqlitePageRefStore};
pub use reasoning_store::{NewClaim, NewDecision, SqliteReasoningStore};
pub use search_store::{sanitize_query, SearchHit, SqliteSearchStore};
pub use semantic_layer::{
    Reads, SchemaColumn, SchemaEntity, SchemaRelation, SemanticLayer, SqlCell, SqlParams, SqlQuery,
    SqlQueryResult,
};
pub use snapshot_tree::{ContentHasher, SnapshotTree, TreeEntry};
pub use stream_store::SqliteStreamStore;
pub use task_satellite::{SqliteTaskLinkStore, SqliteTaskNoteStore};
pub use task_store::{EffortTransition, SqliteTaskStore};
pub use thread_store::SqliteThreadStore;
pub use token_usage_store::{
    AgentTokenUsage, NewAgentTokenUsage, SqliteTokenUsageStore, TokenUsageTotals,
};
pub use tool_call_store::{NewToolCall, SqliteToolCallStore};
pub use wiki_page_store::{SqliteWikiPageStore, WikiPage, WikiPageSearchHit, WikiPageStore};
pub use wiki_page_thread_updates::{SqliteWikiPageThreadUpdateStore, WikiPageThreadUpdate};
