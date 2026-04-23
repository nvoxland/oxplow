# Working in this repo

`.context/` is the project's durable knowledge base. Treat it as the
authoritative place for anything you'd otherwise stash in agent memory —
project decisions, system mechanics, gotchas, conventions, "why we did
it this way" notes.

1. **Read the relevant doc before touching its subsystem.** They're
   short on purpose — skipping them costs more than reading them.
2. **Update the relevant doc in the same commit as your change.** Docs
   that drift from code are worse than no docs.
3. **Capture new knowledge in `.context/`, not in memory.** If you
   discover a non-obvious decision, a recurring gotcha, an undocumented
   convention, or something you'd want to remember next session — write
   it into the matching doc.

## Always-on rules

- **`.context/architecture.md`** — the high-level stance. Don't violate
  the workspace isolation rule without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape cancels,
  drop-target highlighting, right-click for destructive actions, etc.).
  Read before adding *any* UI.

Use plan mode for multi-subsystem work (3+ areas touched) or ambiguous
requirements. Skip it for single-file changes, typos, renames, or narrow
refactors — go straight to TDD or a subagent dispatch.

## Subsystem docs — when to read which

| If you're touching… | Read first |
|---|---|
| Tables, stores, work queue, commit/wait points, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution | `.context/git-integration.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |

When you finish a change that alters how a subsystem works, **update
the matching `.context/` doc in the same commit**. Concrete triggers:

- Added a new table / store / migration → update `data-model.md`.
- Added a new MCP tool, hook, or Stop-hook branch → update `agent-model.md`.
- Added a new IPC method or event type → update `ipc-and-stores.md`.
- Added or repurposed a CSS variable → update `theming.md`.
- Added a new fs watcher or git operation → update `git-integration.md`.
- Changed how the editor pane handles models, menus, or decorations → update `editor-and-monaco.md`.

Docs reference source by **path only** (no line numbers — they drift).

## Tests

Stores have colocated `bun:test` files. Cross-store / Stop-hook / MCP
behavior goes in `src/electron/runtime.test.ts`. Don't mock the DB —
tests use a fresh `mkdtempSync` project dir against a real SQLite file.

## Work items are observational

Newde passively tracks active agent turns: each open `agent_turn` row
(`ended_at IS NULL` and started after runtime boot) renders as a live
row in the Work panel's in_progress bucket showing the prompt,
"thinking…", and elapsed time. When the turn Stops, the row
disappears. No synthesized work items, no auto-file/auto-complete, no
adoption — you don't need to narrate turn boundaries.

**File a durable work item before you start editing.** When you realize
you're about to change project files in a turn and you aren't already
working against an existing item, call
`mcp__newde__create_work_item` (or `file_epic_with_children` if the
work is large enough to be worth splitting into macro subtasks) with
status `in_progress` and track your progress against it across however
many turns it takes — including stops to ask the user questions. The
item should describe the real piece of work you're committing to
shipping, not a placeholder "auto" row that may or may not get
reshaped into something real. When it's settled, call `complete_task`
to ship an explicit summary.

Claude Code's built-in TaskCreate/TaskUpdate is captured to
`agent_turn.task_list_json` on every call and rendered live on the
open-turn row as an expandable sub-list. It stays out of the
persistent work-item stream.
