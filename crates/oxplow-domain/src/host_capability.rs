//! Host capabilities: the native things a command's handler may do —
//! read the semantic layer, write oxplow's records, run git, write files,
//! open a dialog — each named like an OAuth scope (resource + action) and
//! declared in the command's `needs` (`.context/commands.md` "Host
//! capabilities"). Every command — oxplow's own and any extension's —
//! reaches them the same way; a call to one the command didn't declare is
//! refused.
//!
//! Not to be confused with [`crate::capability`]: the swappable pieces a
//! project chooses an implementation of (`work_items`, `snapshots`). A
//! command's `needs` may name both; each name is looked up in its own
//! catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a capability does to the world: a command's effect is the
/// strongest class among what it needs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    /// Changes only what the person sees (open a page, find in a file).
    View,
    /// Reads, changes nothing (`sql.read`).
    Read,
    /// Changes oxplow's own records, in the run's transaction.
    Record,
    /// Changes something outside oxplow's records — files, git, processes —
    /// so it can't run in the run's transaction.
    Write,
}

impl EffectClass {
    /// Whether it runs in the command's database transaction (rolled back
    /// with it): a command needing only these runs as one transaction;
    /// one needing any other runs as steps.
    pub fn in_transaction(self) -> bool {
        matches!(self, EffectClass::Read | EffectClass::Record)
    }

    /// Whether a run that used it is recorded (an audit row and
    /// `command.executed`).
    pub fn audited(self) -> bool {
        matches!(self, EffectClass::Record | EffectClass::Write)
    }
}

/// One host capability, as core declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapability {
    /// `sql.read`: `<resource>.<action>`.
    pub id: &'static str,
    pub class: EffectClass,
    /// What it lets a handler do, for a person deciding to allow it.
    pub summary: &'static str,
}

/// Every host capability core provides.
pub const HOST_CAPABILITIES: &[HostCapability] = &[HostCapability {
    id: "sql.read",
    class: EffectClass::Read,
    summary: "Read oxplow's published models (`v_*`) with SQL: one read-only statement a call, \
              bound by named parameters.",
}];

/// The host capability `id`, if core provides it.
pub fn host_capability(id: &str) -> Option<&'static HostCapability> {
    HOST_CAPABILITIES.iter().find(|c| c.id == id)
}

/// The strongest class among `needs`' host capabilities; `None` when it
/// names none (a command that only composes others).
pub fn strongest_class(needs: &[String]) -> Option<EffectClass> {
    needs
        .iter()
        .filter_map(|n| host_capability(n))
        .map(|c| c.class)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_capability_is_declared_once_as_resource_and_action() {
        let mut ids: Vec<&str> = HOST_CAPABILITIES.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), HOST_CAPABILITIES.len());
        for c in HOST_CAPABILITIES {
            assert_eq!(c.id.split('.').count(), 2, "{}", c.id);
            assert!(!c.summary.is_empty(), "{}", c.id);
        }
    }

    #[test]
    fn a_commands_class_is_the_strongest_it_needs() {
        let needs = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            strongest_class(&needs(&["sql.read"])),
            Some(EffectClass::Read)
        );
        assert_eq!(strongest_class(&needs(&["work_items"])), None);
        assert!(EffectClass::Read.in_transaction() && !EffectClass::Read.audited());
        assert!(EffectClass::Record.in_transaction() && EffectClass::Record.audited());
        assert!(!EffectClass::Write.in_transaction() && EffectClass::Write.audited());
        assert!(!EffectClass::View.in_transaction() && !EffectClass::View.audited());
    }
}
