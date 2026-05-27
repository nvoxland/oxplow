//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod agent_stores;
pub mod analytics_stores;
pub mod comment_store;
mod database;
pub mod effort_store;
pub mod observation_store;
pub mod page_ref_projections;
pub mod page_ref_store;
pub mod search_store;
mod stream_store;
pub mod task_satellite;
pub mod task_store;
mod thread_store;
pub mod wiki_page_store;
pub mod wiki_page_thread_updates;

pub use agent_stores::SqliteAgentTurnStore;
pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, Snapshot, SnapshotChangeEntry, SnapshotStats, SqliteCodeQualityStore,
    SqlitePageVisitStore, SqliteSnapshotStore, SqliteUsageStore, UsageEvent, UsageRollup,
};
pub use comment_store::SqliteCommentStore;
pub use database::{Database, DbInitError};
pub use effort_store::{
    EffortAtSnapshot, EffortFile, EffortFileChange, FileRefVersion, SqliteTaskEffortStore,
    TaskEffort, TaskEffortStore,
};
pub use observation_store::{
    EffortObservation, NewEffortObservation, SqliteEffortObservationStore,
};
pub use page_ref_store::{PageRefEdge, PageRefStore, SqlitePageRefStore};
pub use search_store::{sanitize_query, SearchHit, SqliteSearchStore};
pub use stream_store::SqliteStreamStore;
pub use task_satellite::{SqliteTaskEventStore, SqliteTaskLinkStore, SqliteTaskNoteStore};
pub use task_store::SqliteTaskStore;
pub use thread_store::SqliteThreadStore;
pub use wiki_page_store::{SqliteWikiPageStore, WikiPage, WikiPageSearchHit, WikiPageStore};
pub use wiki_page_thread_updates::{SqliteWikiPageThreadUpdateStore, WikiPageThreadUpdate};
