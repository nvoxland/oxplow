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

/// Where a capability's handler lives: a command it backs runs there,
/// and a call from anywhere else is a round trip to it (VS Code's model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    /// The project's daemon: oxplow's records, the repository, the
    /// project's files.
    Daemon,
    /// The project's window: its threads' tabs, the editor, the search
    /// box, the agent's input.
    Window,
    /// The app shell: projects and windows (New / Open Project). The
    /// window runs its commands by calling the shell; a run on the daemon
    /// reaches the shell through the window.
    Shell,
}

/// One host capability, as core declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapability {
    /// `sql.read`: `<resource>.<action>`.
    pub id: &'static str,
    pub class: EffectClass,
    pub host: Host,
    /// What it lets a handler do, for a person deciding to allow it.
    pub summary: &'static str,
}

/// Every host capability core provides, by id.
pub const HOST_CAPABILITIES: &[HostCapability] = &[
    HostCapability {
        id: "agent_input.write",
        class: EffectClass::View,
        host: Host::Window,
        summary: "Put text in a thread's agent input, unsent — a person's: oxplow never types for the agent.",
    },
    HostCapability {
        id: "bookmarks.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Star a page for a person, at a thread, stream or project, or take the star off.",
    },
    HostCapability {
        id: "collectors.sync",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Run the project's and extensions' collectors now, syncing what they read.",
    },
    HostCapability {
        id: "config.read",
        class: EffectClass::Read,
        host: Host::Daemon,
        summary: "Read the project's configuration (`.oxplow/project.yaml`) and its keys.",
    },
    HostCapability {
        id: "config.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Change the project's configuration file (`.oxplow/project.yaml`).",
    },
    HostCapability {
        id: "dashboards.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Create, rename, delete and arrange dashboards and their tiles.",
    },
    HostCapability {
        id: "diagnostics.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Record an error the window ran into, for the person to see.",
    },
    HostCapability {
        id: "editor.write",
        class: EffectClass::View,
        host: Host::Window,
        summary: "Save a file's unsaved changes in a thread's editor.",
    },
    HostCapability {
        id: "effects.read",
        class: EffectClass::Read,
        host: Host::Daemon,
        summary: "Read what an extension's effect would react to.",
    },
    HostCapability {
        id: "effects.run",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Run an extension's effect again: retry a reaction, backfill past events.",
    },
    HostCapability {
        id: "efforts.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary:
            "Open, update, link and close efforts; record and review their claims and decisions.",
    },
    HostCapability {
        id: "extensions.enable",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Turn a disabled plugin contribution back on.",
    },
    HostCapability {
        id: "extensions.install",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Install or update an extension in the project's `oxplow/extensions/`.",
    },
    HostCapability {
        id: "files.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Write files in a worktree (restore one from a snapshot).",
    },
    HostCapability {
        id: "hints.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Dismiss a hint oxplow showed.",
    },
    HostCapability {
        id: "knowledge.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Write the project's wiki: pages, notes, comments and links.",
    },
    HostCapability {
        id: "lenses.show",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Show a lens beside the conversation.",
    },
    HostCapability {
        id: "lenses.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Keep or share a lens as a file in the project.",
    },
    HostCapability {
        id: "lsp.install",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Install or remove a language server on this machine.",
    },
    HostCapability {
        id: "metrics.read",
        class: EffectClass::Read,
        host: Host::Daemon,
        summary: "Read a metric's definition, scaffolded for editing.",
    },
    HostCapability {
        id: "metrics.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Enable, record and rebuild metrics (changes the configuration and the facts).",
    },
    HostCapability {
        id: "projects.write",
        class: EffectClass::Write,
        host: Host::Shell,
        summary: "Create a project in a folder, or open one — in this window or a new one.",
    },
    HostCapability {
        id: "providers.sync",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Sync a provider's records into oxplow.",
    },
    HostCapability {
        id: "sql.read",
        class: EffectClass::Read,
        host: Host::Daemon,
        summary:
            "Read oxplow's published models (`v_*`) with SQL: one read-only statement a call, \
              bound by named parameters.",
    },
    HostCapability {
        id: "streams.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Rename a stream and set its standing prompt.",
    },
    HostCapability {
        id: "tabs.write",
        class: EffectClass::View,
        host: Host::Window,
        summary: "Open, close and focus tabs in a thread's set of tabs — an agent's only in its own thread's.",
    },
    HostCapability {
        id: "test_runs.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Record a test run's results.",
    },
    HostCapability {
        id: "threads.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Create, rename, reorder, promote, close and reopen threads.",
    },
    HostCapability {
        id: "vcs.remote",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Fetch, pull and push the repository's branches.",
    },
    HostCapability {
        id: "vcs.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Change the repository: stage, commit, discard, branch, merge, rebase, resolve.",
    },
    HostCapability {
        id: "window.show",
        class: EffectClass::View,
        host: Host::Window,
        summary: "Show the window's search box (Quick Open) or its find-in-file bar.",
    },
    HostCapability {
        id: "work_items.write",
        class: EffectClass::Record,
        host: Host::Daemon,
        summary: "Create, update, move, link, comment on and delete work items.",
    },
    HostCapability {
        id: "worktrees.write",
        class: EffectClass::Write,
        host: Host::Daemon,
        summary: "Create, adopt and archive a stream's worktree.",
    },
];

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
        let listed = ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids, listed, "sorted, each once");
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
