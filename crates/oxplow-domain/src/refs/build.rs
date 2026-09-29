//! Canonical ref builders (P2.4b, tsk450; `.context/refs.md`).
//!
//! Every producer that names an entity in an event subject, a payload or
//! a `page_ref` row builds the ref here, so the shape (`stream:str1`,
//! `turn:trn3`, `work_item:oxplow:tsk42`) lives in one place instead of a
//! `format!` per call site. The inverse for oxplow's own work items is
//! here too.

use crate::ids::{AgentTurnId, EffortId, StreamId, TaskId, ThreadId};

/// The provider oxplow's own tasks are filed under in a `work_item` ref.
pub const OXPLOW_PROVIDER: &str = "oxplow";

pub fn stream_ref(id: StreamId) -> String {
    format!("stream:{id}")
}

pub fn thread_ref(id: ThreadId) -> String {
    format!("thread:{id}")
}

pub fn effort_ref(id: EffortId) -> String {
    format!("effort:{id}")
}

pub fn turn_ref(id: AgentTurnId) -> String {
    format!("turn:{id}")
}

/// A snapshot (`snapshot.id`).
pub fn snapshot_ref(id: i64) -> String {
    format!("snapshot:{id}")
}

/// A commit by sha (7–40 lowercase hex).
pub fn commit_ref(sha: &str) -> String {
    format!("commit:{sha}")
}

/// A command by name (`config.set`).
pub fn command_ref(name: &str) -> String {
    format!("command:{name}")
}

/// A `.oxplow/project.yaml` key (`zones`, `metricRetentionDays`).
pub fn config_ref(key: &str) -> String {
    format!("config:{key}")
}

/// The `work_item` id of an oxplow task: `oxplow:tsk<n>` (the id part of a
/// `work_item:` ref, as `page_ref.target_id` stores it).
pub fn work_item_id(task: TaskId) -> String {
    format!("{OXPLOW_PROVIDER}:{task}")
}

/// The canonical ref of an oxplow task: `work_item:oxplow:tsk<n>`.
pub fn work_item_ref(task: TaskId) -> String {
    format!("work_item:{}", work_item_id(task))
}

/// The task behind a `work_item` **id** when it's one of ours: `oxplow:tsk<n>`,
/// or — as an agent's impact declaration may write it — a bare `tsk<n>` / `<n>`.
pub fn task_from_work_item_id(id: &str) -> Option<TaskId> {
    let native = id.strip_prefix("oxplow:").unwrap_or(id);
    TaskId::try_from_str(native).or_else(|| native.parse::<i64>().ok().map(TaskId::new))
}

/// The task behind a canonical `work_item:` **ref**: `Some` for
/// `work_item:oxplow:tsk<n>`, `None` for another provider's item or any
/// other kind (strict: this is what event consumers read).
pub fn task_of_work_item_ref(r: &str) -> Option<TaskId> {
    let id = r.strip_prefix("work_item:")?;
    let native = id.strip_prefix("oxplow:")?;
    TaskId::try_from_str(native)
}

/// The `source` of an event a system component emits on its own behalf
/// (`system:snapshot_capture`); an actor's runs use `Actor::source()`.
pub fn system_source(component: &str) -> String {
    format!("system:{component}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::grammar::CanonicalRef;
    use crate::refs::kind::core_kinds;

    #[test]
    fn every_builder_produces_a_valid_canonical_ref() {
        let reg = core_kinds();
        for r in [
            stream_ref(StreamId::new(1)),
            thread_ref(ThreadId::new(3)),
            effort_ref(EffortId::new(12)),
            turn_ref(AgentTurnId::new(7)),
            snapshot_ref(42),
            commit_ref("4c44d495"),
            command_ref("config.set"),
            config_ref("metricRetentionDays"),
            work_item_ref(TaskId::new(42)),
        ] {
            let parsed = CanonicalRef::parse(&r).unwrap_or_else(|e| panic!("{r}: {}", e.reason()));
            reg.validate(&parsed).unwrap_or_else(|e| panic!("{r}: {e}"));
        }
        assert_eq!(stream_ref(StreamId::new(1)), "stream:str1");
        assert_eq!(turn_ref(AgentTurnId::new(7)), "turn:trn7");
        assert_eq!(work_item_ref(TaskId::new(42)), "work_item:oxplow:tsk42");
    }

    #[test]
    fn the_work_item_inverses() {
        assert_eq!(
            task_of_work_item_ref("work_item:oxplow:tsk42"),
            Some(TaskId::new(42))
        );
        assert_eq!(task_of_work_item_ref("work_item:linear:ENG-1"), None);
        assert_eq!(task_of_work_item_ref("effort:eff1"), None);
        assert_eq!(task_of_work_item_ref("work_item:oxplow:42"), None, "strict");
        assert_eq!(
            task_from_work_item_id("oxplow:tsk42"),
            Some(TaskId::new(42))
        );
        assert_eq!(task_from_work_item_id("tsk42"), Some(TaskId::new(42)));
        assert_eq!(task_from_work_item_id("42"), Some(TaskId::new(42)));
        assert_eq!(system_source("hook_ingest"), "system:hook_ingest");
    }
}
