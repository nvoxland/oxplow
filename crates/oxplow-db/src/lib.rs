//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod agent_job_store;
pub mod agent_message_store;
pub mod agent_nudge_store;
pub mod agent_session_store;
pub mod agent_stores;
pub mod ai_call_store;
pub mod ai_result_store;
pub mod analytics_stores;
pub mod bookmark_store;
pub mod capability_store;
pub mod catalog_listing_store;
pub mod change_store;
pub mod changes;
pub mod collector_store;
pub mod command_audit_store;
pub mod commit_request_store;
pub mod context_store;
pub mod contribution_health_store;
pub mod contribution_store;
mod database;
pub mod datasets;
pub mod diagnostic_store;
pub mod effect_run_store;
pub mod effect_state_store;
pub mod event_content_store;
pub mod event_log_store;
pub mod event_retention;
pub mod event_type_store;
pub mod extension_config_store;
pub mod extension_deployment_store;
pub mod extension_job_store;
pub mod extension_run_store;
pub mod extension_state_store;
pub mod extension_store;
mod fact_chain;
pub mod fact_store;
pub mod git_store;
pub mod model_store;
pub mod op_error_store;
pub mod page_ref_projections;
pub mod page_ref_store;
pub mod panel_layout_store;
pub mod proposal_store;
pub mod provider_collector_store;
pub mod replaceable_store;
pub mod request_analysis_store;
pub mod resource_store;
pub mod search_store;
pub mod semantic_layer;
pub mod snapshot_tree;
mod sql_functions;
pub mod sql_tokens;
pub mod stream_store;
pub mod symbol_store;
pub mod table_generations;
pub mod terminal_store;
pub mod thread_store;
pub mod token_usage_store;
pub mod tool_call_store;
pub mod ui_preference_store;

pub use agent_nudge_store::{
    AgentNudge, Audience, NewAgentNudge, OnceScope, SqliteAgentNudgeStore,
};
pub use agent_session_store::SqliteAgentSessionStore;
pub use agent_stores::{SqliteAgentStatusStore, SqliteAgentTurnStore};
pub use ai_call_store::{NewAiCall, SqliteAiCallStore};
pub use ai_result_store::{AiResult, NewAiResult, SqliteAiResultStore};
pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, Snapshot, SnapshotChangeEntry, SnapshotContentRef, SnapshotOp, SnapshotStats,
    SnapshotStorage, SqliteCodeQualityStore, SqlitePageVisitStore, SqliteSnapshotStore,
    SqliteUsageStore, StampedSnapshot, TakeOutcome, TakeRecord, UsageEvent,
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
pub use contribution_health_store::{
    ContributionHealthRow, ContributionKey, SqliteContributionHealthStore,
};
pub use database::{
    interactive, map_sql_err, string_to_ts, ts_to_string, Database, DbInitError, DerivedDimension,
    PublicationTransaction, TxError,
};
pub use diagnostic_store::{DiagnosticRow, SqliteDiagnosticStore};
pub use event_log_store::{anchors_for_thread_tx, DeadLetter, EventCtx, SqliteEventLogStore};
pub use fact_store::{
    BatchApply, BatchRows, CaptureGroup, CubeReadRow, Dimension, FactRow, FactSliceKey, Measure,
    Members, MetricCapture, MetricDelta, MetricSpec, NewCubeRow, NewDimension, NewFact, NewMeasure,
    NewMetricCapture, NewMetricSpec, SqliteFactStore, TestCaseResult, TestCaseStat,
};
pub use git_store::{GitBranchRow, GitCommitFileRow, GitCommitRow, GitTagRow, SqliteGitStore};
pub use page_ref_projections::{FileRefVersion, OwnedFileRefVersion};
pub use page_ref_store::{PageRefEdge, PageRefStore, SourceSlice, SqlitePageRefStore};
pub use panel_layout_store::{PanelPlacement, SqlitePanelLayoutStore};
pub use proposal_store::{NewProposal, Proposal, ProposalDecision, SqliteProposalStore};
pub use provider_collector_store::{CollectorState, SqliteProviderCollectorStore};
pub use request_analysis_store::{AnalysisSpan, Observation, SqliteRequestAnalysisStore};
pub use search_store::{sanitize_query, SearchHit, SqliteSearchStore};
pub use semantic_layer::{
    DatasetFreshness, Reads, SemanticLayer, SqlCell, SqlParams, SqlQuery, SqlQueryResult,
    TempTable, TempView,
};
pub use snapshot_tree::{ContentHasher, SnapshotTree, TreeEntry};
pub use stream_store::SqliteStreamStore;
pub use symbol_store::{FileSymbols, SqliteSymbolStore, SymbolCapture, SymbolRow};
pub use terminal_store::{SqliteTerminalStore, Terminal};
pub use thread_store::SqliteThreadStore;
pub use token_usage_store::{
    AgentTokenUsage, NewAgentTokenUsage, SqliteTokenUsageStore, TokenUsageTotals,
};
pub use tool_call_store::{NewToolCall, SqliteToolCallStore};
pub use ui_preference_store::{SqliteUiPreferenceStore, UiPreference};
