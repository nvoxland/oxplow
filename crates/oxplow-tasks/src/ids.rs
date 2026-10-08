//! oxplow's task ids: `tsk<n>` for a task, `lnk<n>` for a link between
//! two — the list's own ids, which its id pattern (`tsk\d+`) declares to
//! core. Each serializes as that string.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Why a string isn't one of these ids.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` isn't a {label} ({prefix}<n>)")]
pub struct IdParseError {
    text: String,
    label: &'static str,
    prefix: &'static str,
}

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal, $label:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(into = "String", try_from = "String")]
        pub struct $name(i64);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Wrap a known rowid. Don't pass `0` — use
            /// [`Self::placeholder`] for "no id yet".
            pub const fn new(value: i64) -> Self {
                Self(value)
            }

            /// The id of a row not yet inserted.
            pub const fn placeholder() -> Self {
                Self(0)
            }

            pub const fn is_placeholder(self) -> bool {
                self.0 == 0
            }

            pub const fn value(self) -> i64 {
                self.0
            }

            /// `Some` for exactly `<prefix><digits>`.
            pub fn try_from_str(s: &str) -> Option<Self> {
                s.parse().ok()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.strip_prefix($prefix)
                    .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|n| n.parse().ok())
                    .map(Self)
                    .ok_or_else(|| IdParseError {
                        text: s.to_string(),
                        label: $label,
                        prefix: $prefix,
                    })
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
    };
}

prefixed_id!(TaskId, "tsk", "task id");
prefixed_id!(TaskLinkId, "lnk", "task-link id");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_id_is_its_prefixed_string() {
        assert_eq!(TaskId::new(38).to_string(), "tsk38");
        let json = serde_json::to_string(&TaskId::new(42)).unwrap();
        assert_eq!(json, "\"tsk42\"");
        assert_eq!(
            serde_json::from_str::<TaskId>(&json).unwrap(),
            TaskId::new(42)
        );
        assert!(serde_json::from_str::<TaskId>("\"thr1\"").is_err());
        assert_eq!(TaskLinkId::new(3).to_string(), "lnk3");
    }

    #[test]
    fn only_its_own_prefix_and_digits_parse() {
        assert_eq!(TaskId::try_from_str("tsk42"), Some(TaskId::new(42)));
        for bad in ["thr42", "", "tsk", "tsk4a", "42", "lnk4"] {
            assert_eq!(TaskId::try_from_str(bad), None, "{bad}");
        }
        assert!(TaskId::placeholder().is_placeholder());
        assert!(!TaskId::new(1).is_placeholder());
    }
}
