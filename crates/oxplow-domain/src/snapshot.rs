//! The snapshot capability's shared vocabulary (P2.2, tsk424;
//! `.context/target-architecture.md` §6.1, `.context/data-model.md`
//! "snapshot_op").

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why a snapshot take happened: one row of the operation log each
/// (`snapshot_op.trigger`) and the `trigger` of `snapshot.taken`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTrigger {
    /// An agent turn ended (Stop / interrupt).
    TurnEnd,
    /// The worktree went quiet with no turn open (human edits).
    Quiet,
    /// An effort opened (its start bracket).
    EffortStart,
    /// An effort closed (its end bracket), including a restart closing
    /// an orphaned effort.
    EffortEnd,
    /// The boot sweep.
    Startup,
    /// An explicit request (metric baseline rebuild, tests).
    Manual,
    /// HEAD or a ref moved; the take drains whatever was dirty.
    GitRefs,
    /// HEAD moved on a clean tree: the latest snapshot now also is the
    /// new commit (a re-stamp, no new snapshot).
    HeadMoved,
    /// Backfilled for a snapshot taken before the operation log existed.
    Legacy,
}

impl SnapshotTrigger {
    /// The `snapshot_op.trigger` text.
    pub fn as_db_str(self) -> &'static str {
        match self {
            SnapshotTrigger::TurnEnd => "turn_end",
            SnapshotTrigger::Quiet => "quiet",
            SnapshotTrigger::EffortStart => "effort_start",
            SnapshotTrigger::EffortEnd => "effort_end",
            SnapshotTrigger::Startup => "startup",
            SnapshotTrigger::Manual => "manual",
            SnapshotTrigger::GitRefs => "git_refs",
            SnapshotTrigger::HeadMoved => "head_moved",
            SnapshotTrigger::Legacy => "legacy",
        }
    }

    pub fn from_db_str(s: &str) -> Option<SnapshotTrigger> {
        Some(match s {
            "turn_end" => SnapshotTrigger::TurnEnd,
            "quiet" => SnapshotTrigger::Quiet,
            "effort_start" => SnapshotTrigger::EffortStart,
            "effort_end" => SnapshotTrigger::EffortEnd,
            "startup" => SnapshotTrigger::Startup,
            "manual" => SnapshotTrigger::Manual,
            "git_refs" => SnapshotTrigger::GitRefs,
            "head_moved" => SnapshotTrigger::HeadMoved,
            "legacy" => SnapshotTrigger::Legacy,
            _ => return None,
        })
    }

    pub const ALL: [SnapshotTrigger; 9] = [
        SnapshotTrigger::TurnEnd,
        SnapshotTrigger::Quiet,
        SnapshotTrigger::EffortStart,
        SnapshotTrigger::EffortEnd,
        SnapshotTrigger::Startup,
        SnapshotTrigger::Manual,
        SnapshotTrigger::GitRefs,
        SnapshotTrigger::HeadMoved,
        SnapshotTrigger::Legacy,
    ];
}

/// The canonical ref of a snapshot (`snapshot:123`).
pub fn snapshot_ref(id: i64) -> String {
    format!("snapshot:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_text_round_trips_and_matches_serde() {
        for t in SnapshotTrigger::ALL {
            assert_eq!(SnapshotTrigger::from_db_str(t.as_db_str()), Some(t));
            assert_eq!(
                serde_json::to_value(t).unwrap(),
                serde_json::Value::String(t.as_db_str().into())
            );
        }
        assert_eq!(SnapshotTrigger::from_db_str("nope"), None);
        assert_eq!(snapshot_ref(12), "snapshot:12");
    }
}
