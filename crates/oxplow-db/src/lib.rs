//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod agent_nudge_store;
pub mod agent_stores;
pub mod ai_call_store;
pub mod ai_result_store;
pub mod analytics_stores;
pub mod bookmark_store;
pub mod capability_store;
pub mod change_store;
pub mod changes;
pub mod collector_store;
pub mod command_audit_store;
pub mod comment_store;
pub mod dashboard_store;
mod database;
pub mod diagnostic_store;
pub mod effect_run_store;
pub mod effect_state_store;
pub mod effort_evidence_store;
pub mod effort_store;
pub mod event_content_store;
pub mod event_log_store;
pub mod event_retention;
pub mod event_type_store;
pub mod fact_store;
pub mod git_store;
pub mod models;
pub mod page_ref_projections;
pub mod page_ref_store;
pub mod panel_layout_store;
pub mod plugin_health_store;
pub mod proposal_store;
pub mod provider_collector_store;
pub mod reasoning_store;
pub mod ref_kind_store;
pub mod search_store;
pub mod semantic_layer;
pub mod snapshot_tree;
pub mod sql_tokens;
pub mod stream_store;
pub mod symbol_store;
pub mod table_generations;
#[cfg(test)]
pub(crate) mod test_tasks;
pub mod thread_answer_store;
pub mod thread_note_store;
pub mod thread_store;
pub mod token_usage_store;
pub mod tool_call_store;
pub mod wiki_page_store;
pub mod wiki_page_thread_updates;
pub mod work_item_refs;

pub use agent_nudge_store::{
    AgentNudge, Audience, NewAgentNudge, OnceScope, SqliteAgentNudgeStore,
};
pub use agent_stores::{SqliteAgentStatusStore, SqliteAgentTurnStore};
pub use ai_call_store::{NewAiCall, SqliteAiCallStore};
pub use ai_result_store::{AiResult, NewAiResult, SqliteAiResultStore};
pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, Snapshot, SnapshotChangeEntry, SnapshotContentRef, SnapshotOp, SnapshotStats,
    SnapshotStorage, SqliteCodeQualityStore, SqlitePageVisitStore, SqliteSnapshotStore,
    SqliteUsageStore, StampedSnapshot, TakeOutcome, TakeRecord, UsageEvent, UsageRollup,
};
pub use capability_store::{CapabilityProvider, SqliteCapabilityStore};
pub use change_store::{
    ChangeDuplicateRow, ChangeFileRow, ChangeFunctionRow, ChangeImportRow, ChangeResults,
    ChangeRow, ChangeTestFileRow, SqliteChangeStore,
};
pub use collector_store::{
    CollectorRun, EntityColumn, EntityTable, EntityWrite, PendingRun, SqliteCollectorStore,
    StoredType,
};
pub use command_audit_store::{CommandAudit, NewCommandAudit, SqliteCommandAuditStore};
pub use comment_store::SqliteCommentStore;
pub use dashboard_store::{
    Dashboard, DashboardItem, DashboardWithItems, NewDashboardItem, SqliteDashboardStore,
};
pub use database::{map_sql_err, string_to_ts, ts_to_string, Database, DbInitError};
pub use diagnostic_store::{DiagnosticRow, SqliteDiagnosticStore};
pub use effort_evidence_store::EffortObservation;
pub use effort_evidence_store::SqliteEffortEvidenceStore;
pub use effort_store::{
    Effort, EffortAtSnapshot, EffortFile, EffortFileChange, EffortStore, FileRefVersion,
    FileSource, OwnedFileRefVersion, SqliteEffortStore,
};
pub use event_log_store::{anchors_for_thread_tx, DeadLetter, EventCtx, SqliteEventLogStore};
pub use fact_store::{
    BatchApply, BatchRows, CubeReadRow, Dimension, EffortMetricDelta, FactRow, FactSliceKey,
    Measure, MetricCapture, MetricSpec, NewCubeRow, NewDimension, NewFact, NewMeasure,
    NewMetricCapture, NewMetricSpec, SqliteFactStore, TestCaseResult, TestCaseStat,
};
pub use git_store::{GitBranchRow, GitCommitFileRow, GitCommitRow, GitTagRow, SqliteGitStore};
pub use page_ref_store::{PageRefEdge, PageRefStore, SourceSlice, SqlitePageRefStore};
pub use panel_layout_store::{PanelPlacement, SqlitePanelLayoutStore};
pub use plugin_health_store::{PluginHealthRow, PluginKey, SqlitePluginHealthStore};
pub use proposal_store::{NewProposal, Proposal, ProposalDecision, SqliteProposalStore};
pub use provider_collector_store::{CollectorState, SqliteProviderCollectorStore};
pub use reasoning_store::{
    record_claim_tx, record_decision_tx, NewClaim, NewDecision, SqliteReasoningStore,
};
pub use search_store::{sanitize_query, SearchHit, SqliteSearchStore};
pub use semantic_layer::{
    ModelFreshness, Reads, SemanticLayer, SqlCell, SqlParams, SqlQuery, SqlQueryResult, TempTable,
    TempView,
};
pub use snapshot_tree::{ContentHasher, SnapshotTree, TreeEntry};
pub use stream_store::SqliteStreamStore;
pub use symbol_store::{FileSymbols, SqliteSymbolStore, SymbolCapture, SymbolRow};
pub use thread_answer_store::{AnswerShows, SqliteThreadAnswerStore, ThreadAnswer};
pub use thread_note_store::SqliteThreadNoteStore;
pub use thread_store::SqliteThreadStore;
pub use token_usage_store::{
    AgentTokenUsage, NewAgentTokenUsage, SqliteTokenUsageStore, TokenUsageTotals,
};
pub use tool_call_store::{NewToolCall, SqliteToolCallStore};
pub use wiki_page_store::{SqliteWikiPageStore, WikiPage};
pub use wiki_page_thread_updates::{SqliteWikiPageThreadUpdateStore, WikiPageThreadUpdate};
