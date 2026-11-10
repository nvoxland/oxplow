//! oxtask as tasks: `oxplow:oxplow/task?id=tsk<n>&provider=oxtask`.

use crate::ids::OxtaskId;

/// The provider oxtask are filed under in a task's URI.
pub const PROVIDER: &str = "oxtask";

/// A task's URI: `oxplow:oxplow/task?id=tsk<n>&provider=oxtask`.
pub fn task_ref(task: OxtaskId) -> String {
    oxplow_domain::uri::build::task_uri(PROVIDER, &task.to_string())
}

/// The task behind a task's URI: `Some` for one of oxtask,
/// `None` for another list's item or any other URI (strict).
pub fn oxtask_of_task_ref(r: &str) -> Option<OxtaskId> {
    let (provider, id) = oxplow_domain::uri::build::task_of_uri(r)?;
    (provider == PROVIDER)
        .then(|| OxtaskId::try_from_str(&id))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tasks_ref_and_its_inverse() {
        use oxplow_domain::uri::build::{keyed, task_uri};
        assert_eq!(
            task_ref(OxtaskId::new(42)),
            "oxplow:oxplow/task?id=tsk42&provider=oxtask"
        );
        assert_eq!(
            oxtask_of_task_ref(&task_uri("oxtask", "tsk42")),
            Some(OxtaskId::new(42))
        );
        assert_eq!(oxtask_of_task_ref(&task_uri("issues", "ENG-1")), None);
        assert_eq!(
            oxtask_of_task_ref(&keyed(oxplow_domain::uri::resources::EFFORT, "eff1")),
            None
        );
        assert_eq!(
            oxtask_of_task_ref(&task_uri("oxtask", "42")),
            None,
            "strict"
        );
        assert_eq!(oxtask_of_task_ref("task:oxplow:tsk42"), None, "an old ref");
    }
}
