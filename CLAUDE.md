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

Newde auto-tracks your work. Auto-file: on the first write-intent tool
call of a turn the runtime synthesizes a work item for the user's
prompt. Auto-complete: at Stop it derives a summary from the diff and
flips the item to `human_check`. See `.context/agent-model.md` for the
full mechanics. You don't need to narrate filings or transitions.

Call `mcp__newde__create_work_item` / `file_epic_with_children` when
you want to split, link, or pre-queue work — the auto-filed row is
adopted in place. Call `complete_task` when you want to ship an
explicit summary that overrides the auto-one.

Claude Code's built-in TaskCreate/TaskUpdate is for intra-turn
micro-planning; its final state is serialized as a note on the
auto-filed item at Stop, so it stays out of the user-visible newde
surface unless you promote it.
