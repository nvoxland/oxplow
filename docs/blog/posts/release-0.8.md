---
date: 2026-11-16
categories:
  - Releases
---

# Oxplow 0.8 -- Working with your process, not steering it

0.8 finishes the push toward an Oxplow that's a base you shape around how you work with your agents. The tracker, the wiki and the way work gets recorded are now pieces you can keep, swap or turn off.

<!-- more -->

Back in 0.5 I called the goal an "Integrated Understanding Environment". Through 0.7, Oxplow still had opinions about your process baked in: efforts that wanted work declared up front, a built-in task list, a built-in wiki. 0.8 takes those out of the core:

- Oxplow records what happened (requests, turns, files, tests) instead of asking you or the agent to declare it.
- Tasks and knowledge are models that any extension can fill. Oxplow's own tracker and wiki are two such extensions, and you can replace either one with whatever you already use.
- How much the agent tracks its own work is a setting, and "off" is one of the choices.
- Every extension and every agent is one switch in Settings.

## Tasks and knowledge

Core now defines a task model and a knowledge model and nothing else. A "store" is an extension that fills one of them, and several can be on at once. Every store's rows show up on the same Tasks and Knowledge pages, and in search, links and metrics.

- **oxtask** is Oxplow's own task tracker. It keeps its tasks in its own SQLite file, never reuses an id, and records whether a person, an agent or Oxplow wrote each task and comment. It has a setting for how much the agent tracks on its own (it files and moves its own tasks, only you file them, or off), and a definition of done.
- **oxwiki** is the wiki, as markdown files in `.oxplow/oxwiki/`. Agents write pages through `oxwiki.page.write`, and Oxplow records which pages an agent reads. A page that cites code is marked stale when that code moves on.
- **Claude Code's memories** of your project show up as a read-only knowledge store, `claude-memory`, so they're searchable and linkable next to the wiki.
- The task screens: a board, a list scoped to this thread, its stream, you, unassigned or everyone, an **Assigned to you** page, and a **Work** panel showing what's in progress and what each task's latest request is doing.
- "Work item" is now "task" everywhere.

Both are on by default. If you'd rather use another tracker, turn oxtask off and have your agent write a store for the one you use. The extension skill covers `models:` and `fills:`.

## Settings

Core and extension settings now share one registry, one set of files and one Settings page per area.

- Three levels: `.oxplow/local.yaml` (you, this checkout), `.oxplow/project.yaml` (the team) and `global.yaml` in your Oxplow config folder. The first level that sets a key wins.
- Keys are namespaced: `oxplow.extensions.oxwiki.enabled`, `oxplow.agents.codex.enabled`, `oxplow.ai.roles.summarize`. One switch per extension and per agent, no more enable and disable lists.
- AI providers are switched on in Settings -> Integrations, with keys in your keychain.
- Old keys and files (`personal.yaml`, `ai.yaml`, `instances.yaml`, the list-style keys) aren't migrated. Oxplow refuses to load them and tells you which file and what replaces it, without printing the values. The [settings reference](../../reference/settings.md) has the full list.

## Requests replace efforts

Efforts are gone. They asked for work to be declared before it started, which is the kind of steering 0.8 is getting away from. The unit of work is now the request: everything that happened between your message and the agent finishing, across turns, threads and subagents. Oxplow works it out from what it observes.

- A request has an address and totals: turns, tool calls, tokens and running jobs. `oxplow_request_summary` gives an agent the whole picture in one read.
- Requests link to the tasks and commits they touched, and the Changes page lists other requests that changed the same files.
- A turn records how it ended (finished, interrupted, the session ended) and the question it ended on, if any.
- Metrics trend per request: tokens, cycle time, test outcome, time to green, and how many file-changing requests had a task.

Decisions, claims and review verdicts went with efforts.

## Extensions

- An extension can declare models and fill them, keep its own SQLite file, and watch folders in the project. It can also read folders outside it, read-only: your home folder, or an agent's own data folder (that's how `claude-memory` works).
- Collectors can run on start, on file changes or on changes to an extension's SQLite file.
- Pages and custom components can call named actions that any store implements, so a New or Delete button works whichever store a row came from.
- Fixtures can test observers, panels and custom components' own code.
- Oxplow only ships `oxplow-foundation` and `oxplow-bundled` now. I took the parts of bundled I hadn't gone through out of the app; they'll come back one at a time once I have.

## Everything else

- Codex and OpenCode got a lot of reliability work: approvals, interrupts, subagents, compaction and errors are all tracked now. A new Codex thread no longer starts an unprompted turn.
- File watching was rebuilt. Each stream has one watch, an edit costs milliseconds whatever the repo's size, and changes made while Oxplow was closed are picked up when it starts.
- The left panels are Go To, Git and Work. The Git panel has an overview and a single file tree.
- A command that fails shows its error next to the button you pressed, with a link to the details.
- `[[` in the editor links to a knowledge page directly instead of also offering the page's file.

About 590 commits since 0.7.2.

## Upgrading from 0.7

Tasks and wiki pages from 0.7 don't carry over: 0.7 kept them in core's tables, and 0.8 drops those tables. The same goes for settings in the old files and keys. Oxplow names each one it refuses, so you can move them by hand.

If you have 0.7 tasks you want to keep, export them before upgrading, or tell me and I'll help.

Let me know what breaks, especially with Codex and OpenCode, and whether oxtask's tracking setting does what you'd expect.
