//! oxplow's tasks as work items: `oxplow:oxplow/work_item?id=tsk<n>&provider=oxplow`.

use crate::ids::TaskId;

/// The provider oxplow's tasks are filed under in a work item's URI.
pub const PROVIDER: &str = "oxplow";

/// A task's URI: `oxplow:oxplow/work_item?id=tsk<n>&provider=oxplow`.
pub fn work_item_ref(task: TaskId) -> String {
    oxplow_domain::uri::build::work_item_uri(PROVIDER, &task.to_string())
}

/// The task behind a work item's URI: `Some` for one of oxplow's tasks,
/// `None` for another list's item or any other URI (strict).
pub fn task_of_work_item_ref(r: &str) -> Option<TaskId> {
    let (provider, id) = oxplow_domain::uri::build::work_item_of_uri(r)?;
    (provider == PROVIDER)
        .then(|| TaskId::try_from_str(&id))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tasks_ref_and_its_inverse() {
        use oxplow_domain::uri::build::{keyed, work_item_uri};
        assert_eq!(
            work_item_ref(TaskId::new(42)),
            "oxplow:oxplow/work_item?id=tsk42&provider=oxplow"
        );
        assert_eq!(
            task_of_work_item_ref(&work_item_uri("oxplow", "tsk42")),
            Some(TaskId::new(42))
        );
        assert_eq!(
            task_of_work_item_ref(&work_item_uri("issues", "ENG-1")),
            None
        );
        assert_eq!(
            task_of_work_item_ref(&keyed(oxplow_domain::uri::resources::EFFORT, "eff1")),
            None
        );
        assert_eq!(
            task_of_work_item_ref(&work_item_uri("oxplow", "42")),
            None,
            "strict"
        );
        assert_eq!(
            task_of_work_item_ref("work_item:oxplow:tsk42"),
            None,
            "an old ref"
        );
    }
}
