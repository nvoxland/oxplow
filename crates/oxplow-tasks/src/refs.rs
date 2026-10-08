//! oxplow's tasks as work items: `work_item:oxplow:tsk<n>`.

use crate::ids::TaskId;

/// The provider oxplow's tasks are filed under in a `work_item` ref.
pub const PROVIDER: &str = "oxplow";

/// The `work_item` id of a task: `oxplow:tsk<n>` (the id part of its ref,
/// as `page_ref` stores it).
pub fn work_item_id(task: TaskId) -> String {
    format!("{PROVIDER}:{task}")
}

/// A task's canonical ref: `work_item:oxplow:tsk<n>`.
pub fn work_item_ref(task: TaskId) -> String {
    format!("work_item:{}", work_item_id(task))
}

/// The task behind a canonical `work_item:` ref: `Some` for
/// `work_item:oxplow:tsk<n>`, `None` for another list's item or any other
/// kind (strict).
pub fn task_of_work_item_ref(r: &str) -> Option<TaskId> {
    let id = r.strip_prefix("work_item:")?;
    let native = id.strip_prefix(&format!("{PROVIDER}:"))?;
    TaskId::try_from_str(native)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tasks_ref_and_its_inverse() {
        assert_eq!(work_item_ref(TaskId::new(42)), "work_item:oxplow:tsk42");
        assert_eq!(
            task_of_work_item_ref("work_item:oxplow:tsk42"),
            Some(TaskId::new(42))
        );
        assert_eq!(task_of_work_item_ref("work_item:issues:ENG-1"), None);
        assert_eq!(task_of_work_item_ref("effort:eff1"), None);
        assert_eq!(task_of_work_item_ref("work_item:oxplow:42"), None, "strict");
    }
}
