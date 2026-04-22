# Working in this repo

`.context/` is the project's durable knowledge base. Treat it as the
authoritative place for anything you'd otherwise stash in agent memory —
project decisions, system mechanics, gotchas, conventions, "why we did
it this way" notes. Three rules, all mandatory:

1. **Read the relevant doc before touching its subsystem.** They're
   short on purpose — skipping them costs more than reading them.
2. **Update the relevant doc in the same commit as your change.** Docs
   that drift from code are worse than no docs. If you rename a file,
   add a store, change a hook, add a CSS variable, etc., the matching
   `.context/` file gets edited in the same diff. No "I'll update the
   docs later." See "Keeping the docs honest" below for the trigger
   list.
3. **Capture new knowledge in `.context/`, not in memory.** If you
   discover a non-obvious decision, a recurring gotcha, an undocumented
   convention, a recipe worth re-using, or anything else you'd want to
   remember next session — write it into the matching doc. If nothing
   fits, create a new doc and add it to the lookup table below. The
   only things that belong in agent memory instead of `.context/` are
   user-personal preferences (this user's style, this user's
   priorities) — everything project-related lives here so it's shared
   across sessions and across collaborators.

## Always-on rules

- **`.context/architecture.md`** — the high-level "Monaco at the core,
  custom shell around it, streams are first-class, never look outside
  the project root" stance. Don't violate the workspace isolation rule
  without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape
  cancels, drop-target highlighting, right-click for destructive
  actions, etc.). Read before adding *any* UI.

## Subsystem docs — when to read which

Read the matching doc **before** starting work in that area:

| If you're touching… | Read first |
|---|---|
| Tables, stores, work queue, commit/wait points, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution, the "agent never calls git" rule | `.context/git-integration.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |

Each doc opens with a one-paragraph "what this doc covers" summary so
you can confirm relevance in seconds.

## Keeping the docs honest

When you finish a change that alters how a subsystem works, **update
the matching `.context/` doc in the same commit**. The docs lose value
fast if they drift from the code.

Concrete triggers:

- Added a new table / store / migration → update `data-model.md`.
- Added a new MCP tool, hook, or Stop-hook branch → update `agent-model.md`.
- Added a new IPC method or event type → update `ipc-and-stores.md`.
- Added or repurposed a CSS variable → update `theming.md`.
- Added a new fs watcher or git operation → update `git-integration.md`.
- Changed how the editor pane handles models, menus, or decorations →
  update `editor-and-monaco.md`.

Docs reference source by **path only** (no line numbers — they drift).
If you rename a file or remove a referenced symbol, fix the doc.

## Tests

Stores have colocated `bun:test` files. Cross-store / Stop-hook / MCP
behavior goes in `src/electron/runtime.test.ts`. Don't mock the DB —
tests use a fresh `mkdtempSync` project dir against a real SQLite file.

## Two task surfaces, different grains

Claude Code's built-in TaskCreate/TaskUpdate and newde's
`mcp__newde__*_work_item` tools both look like "task lists" but serve
different grains:

- **newde work items** — durable, user-visible, cross-session. "Fix
  the wait-point bug." "Reorganize the Work panel." Persisted to
  SQLite, survive restart, user approves and marks done. This is the
  canonical record of what the agent is doing on behalf of the user.
- **Claude Code TaskCreate** — ephemeral, within-turn micro-planning.
  "Read the pipeline file. Write the failing test. Run tests. Update
  the doc." Invisible to the user, discarded when the turn ends.

Pick ONE surface per piece of work — never mirror. If you're reviewing
batch history, that's where newde work items live; if you see
TaskCreate referenced anywhere, it was a within-turn plan, not a
deliverable.

**Every top-level user request that changes the repo gets a newde work
item — no exceptions, even for one-line edits.** File it via
`mcp__newde__newde__create_work_item` before (or as) you start work,
move it to `in_progress` while you're on it, and set it to
`human_check` when done (never self-mark `done`). One item per
top-level request; your own micro-step breakdown stays internal
(TaskCreate or just in your head) — don't mirror those as newde work
items. The user relies on the Work panel's TODO / IN PROGRESS / HUMAN
CHECK sections as the canonical record of agent activity; skipping the
ticket because a change "feels small" erases that visibility. The only
things that don't need a ticket are pure Q&A, read-only investigation,
and discussion that doesn't result in file changes. Load the
`newde-task-filing`, `newde-task-lifecycle`, and `newde-task-dispatch`
skills for the full filing/status/execution protocols (each fires on
its own trigger so only the relevant one loads per turn).
