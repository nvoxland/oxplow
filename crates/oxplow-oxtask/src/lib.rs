//! oxplow's own task list: one implementation of the task interface
//! (`.context/tasks.md`), named by nothing outside it. Its tables
//! (`task`, `oxtask_link`, `oxtask_note`), the store over them, the mapping of
//! its statuses and priority onto the interface, its answers to the verbs
//! and the records they answer with.
//! Everything else reads the interface (`v_task` and its views) and
//! writes through the `oxplow.task.*` commands, whichever list is
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

pub use ids::OxtaskId;
pub use model::{
    Oxtask, OxtaskActorKind, OxtaskAuthor, OxtaskLinkType, OxtaskNote, OxtaskPriority, OxtaskStatus,
};
pub use provider::OxtaskTracker;
pub use refs::{oxtask_of_task_ref, task_ref, PROVIDER};
pub use satellite::{OxtaskNoteStore, SqliteOxtaskNoteStore};
pub use store::{OxtaskStore, SqliteOxtaskStore};
