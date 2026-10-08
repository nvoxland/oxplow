//! Scopes: the native things a command's handler may do — read the
//! semantic layer, write oxplow's records, run git, write files, open a
//! dialog — each named like an OAuth scope (resource + action) and
//! declared in the command's `needs` (`.context/commands.md` "Scopes").
//! Every command — oxplow's own and any extension's — reaches them the
//! same way; a call to one the command didn't declare is refused.
//!
//! Not to be confused with [`crate::capability`]: the swappable slots a
//! project chooses an implementation of (`work_items`, `snapshots`). A
//! command's `needs` may name both; each name is looked up in its own
//! catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::Access;

/// Where a scope's handler lives: a command it backs runs there,
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

/// One scope, as core declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// `sql.read`: `<resource>.<action>`.
    pub id: &'static str,
    /// What it lets a run change: a command's access is the strongest
    /// among the scopes it needs.
    pub access: Access,
    pub host: Host,
    /// What it lets a handler do, for a person deciding to allow it.
    pub summary: &'static str,
}

/// Every scope core provides, by id.
pub const SCOPES: &[Scope] = &[
    Scope {
        id: "agent_input.write",
        access: Access::View,
        host: Host::Window,
        summary: "Put text in a thread's agent input, unsent — a person's: oxplow never types for the agent.",
    },
    Scope {
        id: "agent_sessions.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Open, rename and close a thread's agent sessions. Opening one only adds the slot (the UI starts its process, and nothing types into it); closing one stops its process.",
    },
    Scope {
        id: "bookmarks.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Star a page for a person, at a thread, stream or project, or take the star off.",
    },
    Scope {
        id: "collectors.sync",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Run the project's and extensions' collectors now, syncing what they read.",
    },
    Scope {
        id: "config.read",
        access: Access::Read,
        host: Host::Daemon,
        summary: "Read the project's configuration (`.oxplow/project.yaml`) and its keys.",
    },
    Scope {
        id: "config.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Change the project's configuration file (`.oxplow/project.yaml`).",
    },
    Scope {
        id: "dashboards.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Create, rename, delete and arrange dashboards and their tiles.",
    },
    Scope {
        id: "diagnostics.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Record an error the window ran into, for the person to see.",
    },
    Scope {
        id: "editor.write",
        access: Access::View,
        host: Host::Window,
        summary: "Save a file's unsaved changes in a thread's editor.",
    },
    Scope {
        id: "effects.read",
        access: Access::Read,
        host: Host::Daemon,
        summary: "Read what an extension's effect would react to.",
    },
    Scope {
        id: "effects.run",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Run an extension's effect again: retry a reaction, backfill past events.",
    },
    Scope {
        id: "efforts.write",
        access: Access::Record,
        host: Host::Daemon,
        summary:
            "Open, update, link and close efforts; record and review their claims and decisions.",
    },
    Scope {
        id: "extensions.enable",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Turn a disabled extension contribution back on.",
    },
    Scope {
        id: "extensions.install",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Install or update an extension in the project's `oxplow/extensions/`.",
    },
    Scope {
        id: "files.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Write files in a worktree (restore one from a snapshot).",
    },
    Scope {
        id: "hints.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Dismiss a hint oxplow showed.",
    },
    Scope {
        id: "knowledge.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Write the project's wiki: pages, notes, comments and links.",
    },
    Scope {
        id: "lenses.show",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Show a lens beside the conversation.",
    },
    Scope {
        id: "lenses.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Keep or share a lens as a file in the project.",
    },
    Scope {
        id: "lsp.install",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Install or remove a language server on this machine.",
    },
    Scope {
        id: "metrics.read",
        access: Access::Read,
        host: Host::Daemon,
        summary: "Read a metric's definition, scaffolded for editing.",
    },
    Scope {
        id: "metrics.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Enable, record and rebuild metrics (changes the configuration and the facts).",
    },
    Scope {
        id: "projects.write",
        access: Access::Write,
        host: Host::Shell,
        summary: "Create a project in a folder, or open one — in this window or a new one.",
    },
    Scope {
        id: "providers.sync",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Sync a provider's records into oxplow.",
    },
    Scope {
        id: "sql.read",
        access: Access::Read,
        host: Host::Daemon,
        summary:
            "Read oxplow's published models (`v_*`) with SQL: one read-only statement a call, \
              bound by named parameters.",
    },
    Scope {
        id: "streams.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Rename a stream and set its standing prompt.",
    },
    Scope {
        id: "tabs.write",
        access: Access::View,
        host: Host::Window,
        summary: "Open, close and focus tabs in a thread's set of tabs — an agent's only in its own thread's.",
    },
    Scope {
        id: "test_runs.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Record a test run's results.",
    },
    Scope {
        id: "threads.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Create, rename, reorder, promote, close and reopen threads.",
    },
    Scope {
        id: "vcs.remote",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Fetch, pull and push the repository's branches.",
    },
    Scope {
        id: "vcs.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Change the repository: stage, commit, discard, branch, merge, rebase, resolve.",
    },
    Scope {
        id: "window.show",
        access: Access::View,
        host: Host::Window,
        summary: "Show the window's search box (Quick Open) or its find-in-file bar.",
    },
    Scope {
        id: "work_items.write",
        access: Access::Record,
        host: Host::Daemon,
        summary: "Create, update, move, link, comment on and delete work items.",
    },
    Scope {
        id: "worktrees.write",
        access: Access::Write,
        host: Host::Daemon,
        summary: "Create, adopt and archive a stream's worktree.",
    },
];

/// The scope `id`, if core provides it.
pub fn scope(id: &str) -> Option<&'static Scope> {
    SCOPES.iter().find(|c| c.id == id)
}

/// The strongest access among `needs`' scopes; `None` when it
/// names none (a command that only composes others).
pub fn strongest_access(needs: &[String]) -> Option<Access> {
    needs
        .iter()
        .filter_map(|n| scope(n))
        .map(|c| c.access)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scope_is_declared_once_as_resource_and_action() {
        let mut ids: Vec<&str> = SCOPES.iter().map(|c| c.id).collect();
        let listed = ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids, listed, "sorted, each once");
        for c in SCOPES {
            assert_eq!(c.id.split('.').count(), 2, "{}", c.id);
            assert!(!c.summary.is_empty(), "{}", c.id);
        }
    }

    #[test]
    fn a_commands_access_is_the_strongest_it_needs() {
        let needs = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(strongest_access(&needs(&["sql.read"])), Some(Access::Read));
        assert_eq!(
            strongest_access(&needs(&["sql.read", "tabs.write"])),
            Some(Access::Read)
        );
        assert_eq!(strongest_access(&needs(&["work_items"])), None);
    }
}
