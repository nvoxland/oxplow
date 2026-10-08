//! oxplow's own task list: one implementation of the work-item interface
//! (`.context/work-items.md`), named by nothing outside it. Its tables
//! (`task`, `task_link`, `task_note`), the store over them, the mapping of
//! its statuses and priority onto the interface, and the service.
//! Everything else reads the interface (`v_work_item` and its views) and
//! writes through the `oxplow.work_item.*` commands, whichever list is
//! active.

mod db;
pub mod mapping;
pub mod model;
pub mod refs;
pub mod satellite;
pub mod service;
pub mod store;
pub mod verbs;

pub use model::{
    Task, TaskActorKind, TaskAuthor, TaskLink, TaskLinkType, TaskNote, TaskPriority, TaskStatus,
};
pub use refs::{task_of_work_item_ref, work_item_id, work_item_ref, PROVIDER};
pub use satellite::{SqliteTaskLinkStore, SqliteTaskNoteStore, TaskLinkStore, TaskNoteStore};
pub use service::{CreateTaskInput, TaskService, TaskServiceError, UpdateTaskChanges};
pub use store::{SqliteTaskStore, TaskStore};
