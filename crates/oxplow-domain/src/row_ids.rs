//! Numeric row ids and sizes the command table passes as plain numbers.
//!
//! The entity ids in [`crate::ids`] travel as `"<prefix><int>"` strings.
//! These rows have always been plain integers on the wire (a snapshot's
//! id, an audit row, a dead letter), and the exporter forbids a bare
//! `i64`/`u64` in a command's arguments and results — a field can opt out
//! with `#[specta(type = oxplow_domain::JsNumber)]`, an argument cannot. So each kind is its
//! own transparent newtype: it serializes as the number, exports as
//! TypeScript's `number`, and can't be mixed up with another kind's id.
//! They stay far inside `Number.MAX_SAFE_INTEGER` (row counts and byte
//! sizes of one machine).

use serde::{Deserialize, Serialize};
use specta::Type;

macro_rules! row_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
        )]
        #[serde(from = "i64", into = "i64")]
        #[specta(type = crate::JsNumber)]
        pub struct $name(i64);

        impl From<i64> for $name {
            fn from(value: i64) -> Self {
                Self(value)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> i64 {
                id.0
            }
        }

        impl $name {
            pub const fn new(value: i64) -> Self {
                Self(value)
            }

            pub const fn value(self) -> i64 {
                self.0
            }
        }
    };
}

row_id!(
    /// A `snapshot` row.
    SnapshotId
);
row_id!(
    /// A `file_snapshot` row: one captured file under a snapshot.
    FileSnapshotId
);
row_id!(
    /// A `command_audit` row.
    AuditId
);
row_id!(
    /// A `command_proposal` row: an agent's run waiting on a person.
    ProposalId
);
row_id!(
    /// A parked event's `dead_letter` row.
    DeadLetterId
);
row_id!(
    /// An `ai_call` row.
    AiCallId
);

/// A size in bytes (the blob store's total, say).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(from = "u64", into = "u64")]
#[specta(type = crate::JsNumber)]
pub struct ByteSize(u64);

impl From<u64> for ByteSize {
    fn from(bytes: u64) -> Self {
        Self(bytes)
    }
}

impl From<ByteSize> for u64 {
    fn from(size: ByteSize) -> u64 {
        size.0
    }
}

impl ByteSize {
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// They cross the wire as bare numbers, as the `i64`s they replace did.
    #[test]
    fn they_serialize_as_plain_numbers() {
        assert_eq!(serde_json::to_string(&SnapshotId::new(7)).unwrap(), "7");
        assert_eq!(
            serde_json::from_str::<AuditId>("12").unwrap(),
            AuditId::new(12)
        );
        assert_eq!(serde_json::to_string(&ByteSize::new(4096)).unwrap(), "4096");
        assert_eq!(
            serde_json::from_str::<Vec<SnapshotId>>("[1,2]").unwrap(),
            vec![SnapshotId::new(1), SnapshotId::new(2)]
        );
        assert!(serde_json::from_str::<SnapshotId>("\"snap7\"").is_err());
    }
}
