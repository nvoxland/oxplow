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

Prefer `mcp__oxplow__get_subsystem_doc({ threadId, name })` over a raw
`Read` when you only need the doc body — it's cheap, returns
`{ content, exists }`, and avoids hard-erroring when the doc doesn't
exist yet.

## Always-on rules

- **`.context/architecture.md`** — the high-level stance. Don't violate
  the workspace isolation rule without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape cancels,
  drop-target highlighting, right-click for destructive actions, etc.).
  Read before adding *any* UI.

Use plan mode for multi-subsystem work (3+ areas touched) or ambiguous
requirements. Skip it for single-file changes, typos, renames, or narrow
refactors — go straight to TDD or a subagent dispatch.

**No trivial-edit carve-out for filing.** Every Edit / Write /
MultiEdit / NotebookEdit on project files requires a tracked work
item — typos, single-line CSS tweaks, and one-file fixes included.
Enforcement is a **PreToolUse hook**: when the writer thread has no
`in_progress` item AND no filing call has fired this turn, the edit
tool is denied at the moment it's invoked, not at end-of-turn. File
the item (or flip a ready row to in_progress), then re-issue the edit.
Bash is intentionally exempt — `git merge`, `git pull`, codegen, and
formatters mutate the worktree as a side effect without representing
authored change worth filing. The `.context/` read rule still gets a
soft pass for tiny mechanical edits — just don't skip the work item.

**Asking the user a question.** When your reply ends with a real
clarifying question, A/B/C choice, or any ask where the user owns the
next move, call `mcp__oxplow__await_user({ threadId, question })` and
end your turn. The Stop hook honours this and suppresses every
directive (no dispatch nudge, no audit, no filing-enforcement) until
the user replies. Don't call it for rhetorical asides — only genuine
open questions.

## Subsystem docs — when to read which

| If you're touching… | Read first |
|---|---|
| Tables, stores, work queue, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution | `.context/git-integration.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |
| Code quality scans (lizard / jscpd subprocess + findings store + Code quality panel) | `.context/code-quality.md` |
| Tab store, page chrome, rail HUD (in-flight IA redesign) | `.context/pages-and-tabs.md` |

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

Oxplow passively tracks active agent turns: each open `agent_turn` row
(`ended_at IS NULL` and started after runtime boot) renders as a live
row in the Work panel's in_progress bucket showing the prompt and a
spinner. When the turn Stops, the row disappears. No synthesized work
items, no auto-file/auto-complete, no adoption — you don't need to
narrate turn boundaries.

**File a durable work item before you start editing** (unless the
change qualifies for the trivial-edit carve-out above). When you're
about to change project files in a turn and you aren't already working
against an existing item, file one with status `in_progress`. The item
should describe the real piece of work you're committing to ship, not
a placeholder. When it's settled, call `complete_task` to ship an
explicit summary.

**Pick `create_work_item` (kind defaults to `task`) for one coherent
change**, even if it spans a few files. Use `file_epic_with_children`
only when the work has ≥3 sub-steps a reviewer would naturally
inspect independently — distinct phases, handoffs, or separable
subsystems. The test: could a child close to `done` on its own
and have the user inspect just that piece? If no, it's one task.

**One user-visible concern per ROW.** Independent concerns must be
separate items (sibling tasks or epic children) — a reviewer has to be
able to accept one and push back on another. Test: if a reasonable
reviewer would want to check them independently, they're separate
rows.

**Every new ask gets its own item.** When the user sends a new request
mid-turn, file a new work item rather than silently expanding the
current item's scope. The exception: if the new ask is genuinely a
correction to the same concern (a fix/redo on something you just
shipped to `done`), reopen that item — call `update_work_item`
to flip it back to `in_progress`, redo the work, then `complete_task`
back to `done`. Filing a "Fix what I just did" task
fragments the history.

**Mid-turn user prompts are a new ask boundary.** When a
`<system-reminder>` injects a new user message while you are still
working on something, treat it as a fresh ask — not as more scope for
the current `in_progress` item. Default action: file a new row before
the next edit. Only stay inside the current item if the new prompt is
a direct correction to that exact item; otherwise the rule above
applies. The Work panel must reflect every distinct concern the user
raised, not just the first one. Runtime nudge: a UserPromptSubmit
reminder fires whenever a new prompt arrives and the thread already
has an `in_progress` item from a prior prompt — it points at the open
item and asks you to choose explicitly. Don't ignore it.

**File backlog ideas as you have them.** When you notice a follow-up
worth doing later — a deferred polish item, a TODO surfaced while
finishing something else — file it as a `ready` work item right then.
Don't bury follow-ups in prose at the end of a reply where they'll be
forgotten. The backlog is the durable record; replies are not.

The runtime handles the rest of the state machine for you: tasks
persist across turn boundaries automatically, the Stop hook reminds
you to audit `in_progress` items only when something actually changed,
and the redo-detection hint on `create_work_item` flags when a new
"Fix …" task probably belongs as a reopen instead.
