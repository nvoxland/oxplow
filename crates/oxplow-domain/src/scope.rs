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

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::commands::Access;

/// What a run called, per scope: the per-run summary its audit row
/// records. One per run, shared by the runs nested in it and by the
/// provider calls it makes (a capability verb's `host/call`s); a retried
/// run starts a fresh one.
#[derive(Debug, Default)]
pub struct ScopeTrace(Mutex<BTreeMap<String, u32>>);

impl ScopeTrace {
    /// Count one call of scope `id`.
    pub fn count(&self, id: &str) {
        *self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_string())
            .or_default() += 1;
    }

    /// Calls made before the run, counted with it (an effect's script
    /// runs before its reaction does).
    pub fn add(&self, counts: &BTreeMap<String, u32>) {
        add_counts(
            &mut self.0.lock().unwrap_or_else(|e| e.into_inner()),
            counts.clone(),
        );
    }

    /// Each scope called, and how often.
    pub fn summary(&self) -> BTreeMap<String, u32> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// `from`'s counts added to `into`'s.
pub fn add_counts(into: &mut BTreeMap<String, u32>, from: BTreeMap<String, u32>) {
    for (id, n) in from {
        *into.entry(id).or_default() += n;
    }
}

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
    /// It spends the person's money — a model call on their provider key —
    /// so a tool over it asks before it runs.
    pub spends: bool,
    /// What it lets a handler do, for a person deciding to allow it.
    pub summary: &'static str,
}

/// Every scope core provides, by id.
pub const SCOPES: &[Scope] = &[
    Scope {
        id: "agent_input.write",
        access: Access::View,
        host: Host::Window,
        spends: false,
        summary: "Put text in a thread's agent input, unsent — a person's: oxplow never types for the agent.",
    },
    Scope {
        id: "agent_sessions.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Open, rename and close a thread's agent sessions. Opening one only adds the slot (the UI starts its process, and nothing types into it); closing one stops its process.",
    },
    Scope {
        id: "agent_text.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read oxplow's own agent text: the skills and their references, as the active implementations offer them.",
    },
    Scope {
        id: "ai.compute",
        access: Access::Read,
        host: Host::Daemon,
        spends: true,
        summary: "Ask the configured model a recorded question (`ai_classify`, `ai_score`, \
                  `ai_summarize`, `ai_extract`), spending the person's AI provider key: \
                  only on their approval, and only from a run that may call out.",
    },
    Scope {
        id: "ai.discover",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read declared AI models, capabilities and limits, without credentials or paid probes.",
    },
    Scope {
        id: "ai.settings",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Set the person's AI providers (their keys go to the keychain), remove one, or bind a role to a provider and model.",
    },
    Scope {
        id: "ai.test",
        access: Access::Read,
        host: Host::Daemon,
        spends: true,
        summary: "Make one small call to a configured AI provider to check its key, URL and model, spending the person's key.",
    },
    Scope {
        id: "bookmarks.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Star a page for a person, at a thread, stream or project, or take the star off.",
    },
    Scope {
        id: "changes.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Analyze a commit's change on demand: read its per-file scopes, filling the derived change cache (it changes no record).",
    },
    Scope {
        id: "collectors.preview",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Dry-run a collector from a stream's worktree and read what it would store, storing nothing.",
    },
    Scope {
        id: "collectors.sync",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Run the project's and extensions' collectors now, syncing what they read.",
    },
    Scope {
        id: "config.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the project's configuration (`.oxplow/project.yaml`) and its keys.",
    },
    Scope {
        id: "config.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Change the project's configuration file (`.oxplow/project.yaml`).",
    },
    Scope {
        id: "context.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Put a page or a piece of text in a thread's context basket for its agents, take one out, or empty it.",
    },
    Scope {
        id: "credentials.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Set or clear a credential or secret an extension keeps in the OS keychain: written, never read back, kept out of every record.",
    },
    Scope {
        id: "dashboards.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Create, rename, delete and arrange dashboards and their tiles.",
    },
    Scope {
        id: "deliveries.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Settle an event a consumer couldn't take: retry it, or give up on it.",
    },
    Scope {
        id: "diagnostics.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Record an error the window ran into, for the person to see.",
    },
    Scope {
        id: "editor.write",
        access: Access::View,
        host: Host::Window,
        spends: false,
        summary: "Save a file's unsaved changes in a thread's editor.",
    },
    Scope {
        id: "effects.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read what an extension's effect would react to.",
    },
    Scope {
        id: "effects.run",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Run an extension's effect again: retry a reaction, backfill past events.",
    },
    Scope {
        id: "efforts.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Open, update, link and close efforts; record and review their claims and decisions.",
    },
    Scope {
        id: "extension_config.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the resolved defaults, project and personal configuration for an extension, including provenance.",
    },
    Scope {
        id: "extension_config.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Write an extension's private configuration through the native configuration operation.",
    },
    Scope {
        id: "extensions.check",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Check an extension before it is deployed: validate its source, or preview what it contributes in a throwaway oxplow (it changes no record).",
    },
    Scope {
        id: "extensions.deploy",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Deploy an extension from its source, roll back to an earlier deployment, turn one on or off, remove it, or share it with the team.",
    },
    Scope {
        id: "extensions.enable",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Turn a disabled extension contribution back on.",
    },
    Scope {
        id: "files.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Write files in a worktree (restore one from a snapshot).",
    },
    Scope {
        id: "handoffs.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Hand work from one thread to another, and settle a handoff: take, decline or withdraw it.",
    },
    Scope {
        id: "hints.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Dismiss a hint oxplow showed.",
    },
    Scope {
        id: "jobs.manage",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Stop any extension's durable job: a person's cancel, whichever extension started it.",
    },
    Scope {
        id: "jobs.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read this extension's durable job status and recorded results.",
    },
    Scope {
        id: "jobs.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Record intent for this extension's declared jobs or request their cancellation; execution requires its own current approval.",
    },
    Scope {
        id: "knowledge.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the wiki's comments and the drift of its refs from the code they name.",
    },
    Scope {
        id: "knowledge.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Write the project's wiki: pages, notes, comments and links.",
    },
    Scope {
        id: "lenses.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Run a lens and read what it shows, as text or rows.",
    },
    Scope {
        id: "lenses.show",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Show a lens in the thread, as a tab of it.",
    },
    Scope {
        id: "lenses.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Keep or share a lens as a file in the project.",
    },
    Scope {
        id: "lsp.control",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Restart a stream's language server session.",
    },
    Scope {
        id: "lsp.install",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Install or remove a language server on this machine.",
    },
    Scope {
        id: "lsp.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Ask a stream's language servers for definitions, references, hover, call hierarchy and symbols.",
    },
    Scope {
        id: "metrics.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read a metric's definition, scaffolded for editing.",
    },
    Scope {
        id: "metrics.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Enable, record and rebuild metrics (changes the configuration and the facts).",
    },
    Scope {
        id: "pages.author",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Write a page file into an extension's source.",
    },
    Scope {
        id: "pages.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read pages: the one a thread's person has open, what a page shows, what it is made of.",
    },
    Scope {
        id: "pages.show",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Show a page beside the conversation: a page by id, or an inline page of your own.",
    },
    Scope {
        id: "pages.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Press a page's action, as the page acting for you.",
    },
    Scope {
        id: "projects.write",
        access: Access::Write,
        host: Host::Shell,
        spends: false,
        summary: "Create a project in a folder, or open one — in this window or a new one.",
    },
    Scope {
        id: "providers.sync",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Sync a provider's records into oxplow.",
    },
    Scope {
        id: "providers.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Add, configure, check, turn on or off, and remove an instance of an extension's provider on this machine.",
    },
    Scope {
        id: "records.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read any record, file at a revision or recorded body by its ref.",
    },
    Scope {
        id: "search.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Search the project: file names, symbols and text across a stream.",
    },
    Scope {
        id: "sql.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read oxplow's published models (`v_*`) with SQL: one read-only statement a call, \
              bound by named parameters.",
    },
    Scope {
        id: "state.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the calling extension's private workflow state or rebuildable cache.",
    },
    Scope {
        id: "state.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Atomically replace the calling extension's own private state or cache with a version/revision check.",
    },
    Scope {
        id: "streams.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Rename a stream and set its standing prompt.",
    },
    Scope {
        id: "tabs.write",
        access: Access::View,
        host: Host::Window,
        spends: false,
        summary: "Open, close and focus tabs in a thread's set of tabs — an agent's only in its own thread's.",
    },
    Scope {
        id: "terminals.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Open, rename and close a stream's terminals: shells in its worktree.",
    },
    Scope {
        id: "test_runs.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Record a test run's results.",
    },
    Scope {
        id: "threads.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Create, rename, mark, close and reopen threads.",
    },
    Scope {
        id: "tree.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the captured tree a run is anchored to — its files, their syntax \
                  (`ast_query`), metrics and duplicate blocks — as the snapshot holds it.",
    },
    Scope {
        id: "vcs.read",
        access: Access::Read,
        host: Host::Daemon,
        spends: false,
        summary: "Read the repository through its provider: diff, blame, history.",
    },
    Scope {
        id: "vcs.remote",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Fetch, pull and push the repository's branches.",
    },
    Scope {
        id: "vcs.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Change the repository: stage, commit, discard, branch, merge, rebase, resolve.",
    },
    Scope {
        id: "window.show",
        access: Access::View,
        host: Host::Window,
        spends: false,
        summary: "Show the window's search box (Quick Open) or its find-in-file bar.",
    },
    Scope {
        id: "work_items.write",
        access: Access::Record,
        host: Host::Daemon,
        spends: false,
        summary: "Create, update, move, link, comment on and delete work items.",
    },
    Scope {
        id: "worktrees.write",
        access: Access::Write,
        host: Host::Daemon,
        spends: false,
        summary: "Create, adopt and archive a stream's worktree.",
    },
];

/// The scope `id`, if core provides it.
pub fn scope(id: &str) -> Option<&'static Scope> {
    SCOPES
        .iter()
        .find(|c| c.id == crate::service::need_scope(id))
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
