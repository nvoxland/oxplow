//! The snapshot capability's shared vocabulary (P2.2, tsk424;
//! `.context/data-model.md` "snapshot_op"), and its interface
//! ([`SnapshotProvider`]): mark the worktree now, what changed between two
//! points, read a path at a point — the last only an implementation
//! declaring `contents` answers with bytes (`.context/work-tracking.md`
//! "Capabilities").

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{EffortId, StreamId, ThreadId};
use crate::tree_diff::FileChange;
use crate::work_items::ActiveSource;

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
    /// A run's reports were recorded (coverage, analysis): the code it
    /// measured, which their captures are pinned to (tsk883).
    RunMeasured,
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
            SnapshotTrigger::RunMeasured => "run_measured",
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
            "run_measured" => SnapshotTrigger::RunMeasured,
            "legacy" => SnapshotTrigger::Legacy,
            _ => return None,
        })
    }

    pub const ALL: [SnapshotTrigger; 10] = [
        SnapshotTrigger::TurnEnd,
        SnapshotTrigger::Quiet,
        SnapshotTrigger::EffortStart,
        SnapshotTrigger::EffortEnd,
        SnapshotTrigger::Startup,
        SnapshotTrigger::Manual,
        SnapshotTrigger::GitRefs,
        SnapshotTrigger::HeadMoved,
        SnapshotTrigger::RunMeasured,
        SnapshotTrigger::Legacy,
    ];
}

pub use crate::refs::build::snapshot_ref;

/// A take of a stream's worktree, as a caller asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRequest {
    pub stream: StreamId,
    pub trigger: SnapshotTrigger,
    pub thread: Option<ThreadId>,
    /// The `agent_turn` it ends (a turn-end take).
    pub turn: Option<i64>,
    pub effort: Option<EffortId>,
    /// How long the caller waits; a take is never cut short, its overrun
    /// is recorded.
    pub budget: Option<Duration>,
}

impl MarkRequest {
    /// A take of `stream` for `trigger`, with no anchors or budget.
    pub fn new(stream: StreamId, trigger: SnapshotTrigger) -> Self {
        Self {
            stream,
            trigger,
            thread: None,
            turn: None,
            effort: None,
            budget: None,
        }
    }
}

/// What a take recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    /// The snapshot the worktree is at now: a new one, or its parent when
    /// nothing changed.
    pub snapshot: i64,
    pub parent: Option<i64>,
    pub unchanged: bool,
    pub file_count: u64,
    pub over_budget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    /// The snapshot was taken by an implementation that keeps no file
    /// contents (`contents` undeclared): what changed is known, not what it
    /// said.
    #[error("snapshot {snapshot} was taken without file contents{}", provider.as_deref().map(|p| format!(" (by `{p}`)")).unwrap_or_default())]
    NoContents {
        snapshot: i64,
        provider: Option<String>,
    },
    /// Its contents were kept, then pruned by retention.
    #[error("the file's contents at that snapshot have expired")]
    Expired,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Storage(String),
}

/// One snapshot implementation, registered under the id the capability's
/// declaration gives (`oxplow`, `hashes`, a provider's instance).
#[async_trait]
pub trait SnapshotProvider: Send + Sync {
    fn id(&self) -> &str;
    /// Whether what it marks keeps file contents (the `contents` feature).
    fn contents(&self) -> bool;
    /// Take the stream's worktree as it is now; `None` when there is
    /// nothing to record (no snapshot yet and nothing in the tree).
    async fn mark(&self, request: &MarkRequest) -> Result<Option<Marked>, SnapshotError>;
    /// What changed between `from` (`None`: the empty tree) and `to`.
    async fn changed(
        &self,
        stream: StreamId,
        from: Option<i64>,
        to: i64,
    ) -> Result<Vec<FileChange>, SnapshotError>;
    /// The bytes of `path` at `snapshot`.
    async fn read_at(&self, snapshot: i64, path: &str) -> Result<Vec<u8>, SnapshotError>;
}

/// The snapshot implementations, by id, and the one a project uses now
/// (`active`: the capability registry's resolution, which falls back to
/// core's default for this required capability). A catalog reload restates
/// the declared ones ([`Self::set_declared`]) and keeps a registered
/// instance ([`Self::register`]).
pub struct SnapshotRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn SnapshotProvider>>>,
    declared: RwLock<BTreeSet<String>>,
    active: ActiveSource,
}

impl SnapshotRegistry {
    pub fn new(active: ActiveSource) -> Self {
        Self {
            providers: RwLock::default(),
            declared: RwLock::default(),
            active,
        }
    }

    /// Register a running instance; a reload keeps it.
    pub fn register(&self, provider: Arc<dyn SnapshotProvider>) {
        let id = provider.id().to_string();
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, provider);
    }

    pub fn unregister(&self, id: &str) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// The declared implementations are now `providers`: those declared
    /// before and not now go, these replace theirs, a registered instance
    /// stays.
    pub fn set_declared(&self, providers: Vec<Arc<dyn SnapshotProvider>>) {
        let mut map = self.providers.write().unwrap_or_else(|e| e.into_inner());
        let mut declared = self.declared.write().unwrap_or_else(|e| e.into_inner());
        for id in std::mem::take(&mut *declared) {
            map.remove(&id);
        }
        for provider in providers {
            let id = provider.id().to_string();
            declared.insert(id.clone());
            map.insert(id, provider);
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn SnapshotProvider>> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn ids(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// The id the project uses now.
    pub fn active_id(&self) -> String {
        (self.active)()
    }

    /// The implementation the project uses now; `None` only before any is
    /// registered (boot).
    pub fn active(&self) -> Option<Arc<dyn SnapshotProvider>> {
        self.get(&self.active_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);

    #[async_trait]
    impl SnapshotProvider for Named {
        fn id(&self) -> &str {
            self.0
        }
        fn contents(&self) -> bool {
            true
        }
        async fn mark(&self, _: &MarkRequest) -> Result<Option<Marked>, SnapshotError> {
            Err(SnapshotError::Storage("a fake".into()))
        }
        async fn changed(
            &self,
            _: StreamId,
            _: Option<i64>,
            _: i64,
        ) -> Result<Vec<FileChange>, SnapshotError> {
            Ok(Vec::new())
        }
        async fn read_at(&self, _: i64, _: &str) -> Result<Vec<u8>, SnapshotError> {
            Err(SnapshotError::NotFound("a fake".into()))
        }
    }

    /// A reload restates the declared implementations and keeps a
    /// registered instance; the active one is what the resolution names.
    #[test]
    fn the_registry_restates_the_declared_and_follows_the_choice() {
        let chosen = Arc::new(RwLock::new(String::from("oxplow")));
        let read = chosen.clone();
        let r = SnapshotRegistry::new(Arc::new(move || read.read().unwrap().clone()));
        r.set_declared(vec![Arc::new(Named("oxplow")), Arc::new(Named("hashes"))]);
        r.register(Arc::new(Named("acme")));
        assert_eq!(r.ids(), ["acme", "hashes", "oxplow"]);
        assert_eq!(r.active().unwrap().id(), "oxplow");
        *chosen.write().unwrap() = "hashes".into();
        assert_eq!(r.active().unwrap().id(), "hashes");
        r.set_declared(vec![Arc::new(Named("oxplow"))]);
        assert_eq!(r.ids(), ["acme", "oxplow"]);
        assert!(
            r.active().is_none(),
            "hashes is gone until the resolution says otherwise"
        );
        r.unregister("acme");
        assert!(!r.has("acme"));
    }

    #[test]
    fn no_contents_names_who_took_it() {
        let e = SnapshotError::NoContents {
            snapshot: 7,
            provider: Some("hashes".into()),
        };
        assert_eq!(
            e.to_string(),
            "snapshot 7 was taken without file contents (by `hashes`)"
        );
    }

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
