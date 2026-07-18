//! Newtype IDs.
//!
//! Every entity id has the same shape: a fixed 3-letter type prefix plus
//! a SQLite-allocated autoincrement integer, e.g. `tsk38`, `thr21`, `str5`.
//! The prefix makes an id self-typing — an agent (or a human) can tell a
//! thread id from a stream id at a glance, and the MCP boundary can reject
//! "you passed a thread id where a stream id was expected".
//!
//! Internally each id is an `i64` (the raw rowid). The prefix is applied
//! only at the string boundary: `Display`/serde produce `"<prefix><int>"`
//! and parsing reverses it. Distinct newtypes ([`StreamId`], [`ThreadId`],
//! …) give compile-time safety so mismatched ids are a type error;
//! [`AnyId`] is the type-erased "carry an id whose kind is known at
//! runtime" value for polymorphic boundaries (the ref graph, MCP id
//! validation).

use serde::{Deserialize, Serialize};
use specta::Type;
use std::fmt;
use std::str::FromStr;

/// The kind of entity an id refers to. Single source of truth for the
/// 3-letter prefix and the human-readable label of every id type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Stream,
    Thread,
    Task,
    TaskLink,
    Note,
    Effort,
    AgentTurn,
    HookEvent,
    Comment,
    CommentMessage,
    Followup,
    PageVisit,
    UsageEvent,
    Dashboard,
    DashboardItem,
}

impl EntityKind {
    /// Every kind, for reverse lookups.
    pub const ALL: [EntityKind; 15] = [
        EntityKind::Stream,
        EntityKind::Thread,
        EntityKind::Task,
        EntityKind::TaskLink,
        EntityKind::Note,
        EntityKind::Effort,
        EntityKind::AgentTurn,
        EntityKind::HookEvent,
        EntityKind::Comment,
        EntityKind::CommentMessage,
        EntityKind::Followup,
        EntityKind::PageVisit,
        EntityKind::UsageEvent,
        EntityKind::Dashboard,
        EntityKind::DashboardItem,
    ];

    /// The fixed 3-letter prefix that opens every id of this kind.
    pub const fn prefix(self) -> &'static str {
        match self {
            EntityKind::Stream => "str",
            EntityKind::Thread => "thr",
            EntityKind::Task => "tsk",
            EntityKind::TaskLink => "lnk",
            EntityKind::Note => "not",
            EntityKind::Effort => "eff",
            EntityKind::AgentTurn => "trn",
            EntityKind::HookEvent => "hke",
            EntityKind::Comment => "cmt",
            EntityKind::CommentMessage => "cmg",
            EntityKind::Followup => "fup",
            EntityKind::PageVisit => "pgv",
            EntityKind::UsageEvent => "usg",
            EntityKind::Dashboard => "dsh",
            EntityKind::DashboardItem => "dti",
        }
    }

    /// Human-readable label, e.g. `"thread id (thr…)"`. Used in the MCP
    /// "wrong id kind" diagnostic.
    pub const fn label(self) -> &'static str {
        match self {
            EntityKind::Stream => "stream id (str…)",
            EntityKind::Thread => "thread id (thr…)",
            EntityKind::Task => "task id (tsk…)",
            EntityKind::TaskLink => "task-link id (lnk…)",
            EntityKind::Note => "note id (not…)",
            EntityKind::Effort => "effort id (eff…)",
            EntityKind::AgentTurn => "agent-turn id (trn…)",
            EntityKind::HookEvent => "hook-event id (hke…)",
            EntityKind::Comment => "comment id (cmt…)",
            EntityKind::CommentMessage => "comment-message id (cmg…)",
            EntityKind::Followup => "follow-up id (fup…)",
            EntityKind::PageVisit => "page-visit id (pgv…)",
            EntityKind::UsageEvent => "usage-event id (usg…)",
            EntityKind::Dashboard => "dashboard id (dsh…)",
            EntityKind::DashboardItem => "dashboard-item id (dti…)",
        }
    }

    /// Resolve a 3-letter prefix back to its kind.
    pub fn from_prefix(prefix: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.prefix() == prefix)
    }
}

/// Error parsing a `"<prefix><int>"` id string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdParseError(String);

impl IdParseError {
    fn unparseable(s: &str) -> Self {
        Self(format!(
            "`{s}` is not a valid id (expected a 3-letter prefix followed by an integer, e.g. `tsk42`)"
        ))
    }

    fn wrong_kind(expected: EntityKind, actual: EntityKind) -> Self {
        Self(format!(
            "expected a {}, but got a {}",
            expected.label(),
            actual.label()
        ))
    }
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdParseError {}

/// A type-erased id: a kind plus its integer. Use this to pass an id
/// around when the kind is only known at runtime (the ref graph, the MCP
/// id validator). For statically-known ids prefer the concrete newtypes
/// ([`StreamId`], [`ThreadId`], …), which `AnyId` converts to via
/// [`TryFrom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(into = "String", try_from = "String")]
#[specta(type = String)]
pub struct AnyId {
    pub kind: EntityKind,
    pub value: i64,
}

impl AnyId {
    pub const fn new(kind: EntityKind, value: i64) -> Self {
        Self { kind, value }
    }
}

impl fmt::Display for AnyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.kind.prefix(), self.value)
    }
}

impl FromStr for AnyId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Need at least the 3-char prefix plus one digit, and the prefix
        // must sit on a char boundary (defends against multibyte input).
        if s.len() >= 4 && s.is_char_boundary(3) {
            let (prefix, rest) = s.split_at(3);
            if let Some(kind) = EntityKind::from_prefix(prefix) {
                if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(value) = rest.parse::<i64>() {
                        return Ok(AnyId::new(kind, value));
                    }
                }
            }
        }
        Err(IdParseError::unparseable(s))
    }
}

impl From<AnyId> for String {
    fn from(id: AnyId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for AnyId {
    type Error = IdParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Generate a distinct `i64`-backed id newtype bound to an [`EntityKind`].
///
/// Each type serializes as the string `"<prefix><int>"`, gives compile-
/// time safety against id mix-ups, and exposes the same surface as the
/// hand-written [`TaskId`] used to: `new`/`value`/`placeholder`/
/// `is_placeholder`/`try_from_str`.
macro_rules! id_type {
    ($name:ident, $kind:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
        #[serde(into = "String", try_from = "String")]
        #[specta(type = String)]
        pub struct $name(i64);

        impl $name {
            /// The entity kind this id refers to.
            pub const KIND: EntityKind = EntityKind::$kind;

            /// Wrap a known rowid (the one SQLite just assigned, or one
            /// received from a prior fetch). Don't pass `0` — use
            /// [`Self::placeholder`] for "no id yet".
            pub const fn new(value: i64) -> Self {
                Self(value)
            }

            /// The "I'm about to be inserted" sentinel (rowid `0`, which
            /// SQLite `AUTOINCREMENT` never issues). Used by upsert IPC to
            /// distinguish "allocate one" from "update this row".
            pub const fn placeholder() -> Self {
                Self(0)
            }

            /// True iff this is the placeholder sentinel.
            pub const fn is_placeholder(self) -> bool {
                self.0 == 0
            }

            /// The raw integer rowid.
            pub const fn value(self) -> i64 {
                self.0
            }

            /// Erase to a runtime-tagged [`AnyId`].
            pub const fn as_any(self) -> AnyId {
                AnyId::new(Self::KIND, self.0)
            }

            /// Parse `"<prefix><int>"`, requiring the prefix to match this
            /// id's kind. Returns `None` on any mismatch.
            pub fn try_from_str(s: &str) -> Option<Self> {
                match AnyId::from_str(s) {
                    Ok(a) if a.kind == Self::KIND => Some(Self(a.value)),
                    _ => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", Self::KIND.prefix(), self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let any = AnyId::from_str(s)?;
                Self::try_from(any)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.to_string()
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdParseError;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }

        impl TryFrom<AnyId> for $name {
            type Error = IdParseError;

            fn try_from(any: AnyId) -> Result<Self, Self::Error> {
                if any.kind == Self::KIND {
                    Ok(Self(any.value))
                } else {
                    Err(IdParseError::wrong_kind(Self::KIND, any.kind))
                }
            }
        }

        impl From<$name> for AnyId {
            fn from(id: $name) -> AnyId {
                id.as_any()
            }
        }
    };
}

id_type!(StreamId, Stream);
id_type!(ThreadId, Thread);
id_type!(TaskId, Task);
id_type!(TaskLinkId, TaskLink);
id_type!(NoteId, Note);
id_type!(EffortId, Effort);
id_type!(AgentTurnId, AgentTurn);
id_type!(HookEventId, HookEvent);
id_type!(CommentId, Comment);
id_type!(CommentMessageId, CommentMessage);
id_type!(FollowupId, Followup);
id_type!(PageVisitId, PageVisit);
id_type!(UsageEventId, UsageEvent);
id_type!(DashboardId, Dashboard);
id_type!(DashboardItemId, DashboardItem);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_prefix_and_int() {
        assert_eq!(StreamId::new(5).to_string(), "str5");
        assert_eq!(ThreadId::new(21).to_string(), "thr21");
        assert_eq!(TaskId::new(38).to_string(), "tsk38");
    }

    #[test]
    fn serializes_as_prefixed_string() {
        let json = serde_json::to_string(&TaskId::new(42)).unwrap();
        assert_eq!(json, "\"tsk42\"");
        let back: TaskId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TaskId::new(42));
    }

    #[test]
    fn round_trips_serde() {
        let id = StreamId::new(7);
        let json = serde_json::to_string(&id).unwrap();
        let back: StreamId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn try_from_str_requires_matching_prefix() {
        assert_eq!(TaskId::try_from_str("tsk42"), Some(TaskId::new(42)));
        // A thread id is not a task id.
        assert_eq!(TaskId::try_from_str("thr42"), None);
        assert_eq!(TaskId::try_from_str(""), None);
        assert_eq!(TaskId::try_from_str("tsk"), None);
        assert_eq!(TaskId::try_from_str("tsk4a"), None);
        assert_eq!(TaskId::try_from_str("42"), None);
    }

    #[test]
    fn deserialize_rejects_wrong_kind() {
        let err = serde_json::from_str::<TaskId>("\"thr1\"");
        assert!(err.is_err());
    }

    #[test]
    fn placeholder_round_trips_through_predicate() {
        assert!(TaskId::placeholder().is_placeholder());
        assert!(!TaskId::new(1).is_placeholder());
    }

    #[test]
    fn any_id_parses_and_routes_by_prefix() {
        let any: AnyId = "eff9".parse().unwrap();
        assert_eq!(any.kind, EntityKind::Effort);
        assert_eq!(any.value, 9);
        assert_eq!(any.to_string(), "eff9");
        // Downcast to the matching newtype succeeds; mismatched fails.
        assert_eq!(EffortId::try_from(any), Ok(EffortId::new(9)));
        assert!(StreamId::try_from(any).is_err());
    }

    #[test]
    fn entity_kind_prefixes_are_unique_and_three_chars() {
        let mut seen = std::collections::HashSet::new();
        for kind in EntityKind::ALL {
            assert_eq!(kind.prefix().len(), 3, "{kind:?} prefix must be 3 chars");
            assert!(
                seen.insert(kind.prefix()),
                "duplicate prefix {}",
                kind.prefix()
            );
            assert_eq!(EntityKind::from_prefix(kind.prefix()), Some(kind));
        }
    }
}
