//! oxplow's own task list: one implementation of the work-item interface
//! (`.context/work-items.md`), named by nothing outside it. Its tables
//! (`task`, `task_link`, `task_note`), the store over them, the mapping of
//! its statuses and priority onto the interface, its answers to the verbs
//! and the records they answer with.
//! Everything else reads the interface (`v_work_item` and its views) and
//! writes through the `oxplow.work_item.*` commands, whichever list is
//! active.

mod db;
pub mod ids;
pub mod mapping;
pub mod model;
pub mod provider;
pub mod record;
pub mod refs;
pub mod satellite;
pub mod store;
pub mod verbs;

pub use ids::{TaskId, TaskLinkId};
pub use model::{
    Task, TaskActorKind, TaskAuthor, TaskLink, TaskLinkType, TaskNote, TaskPriority, TaskStatus,
};
pub use provider::OxplowTasks;
pub use refs::{task_of_work_item_ref, work_item_id, work_item_ref, PROVIDER};
pub use satellite::{SqliteTaskLinkStore, SqliteTaskNoteStore, TaskLinkStore, TaskNoteStore};
pub use store::{SqliteTaskStore, TaskStore};
