---
name: oxplow-runtime
description: Oxplow runtime — task filing, status transitions, and orchestrator dispatch. Loads on mcp__oxplow__create_task, file_epic_with_children, update_task, add_thread_note, read_task_options, or dispatch_task calls, and when composing a subagent brief.
---

# Filing oxplow tasks

Active agent turns render as live rows in the Work panel passively —
no synthesized tasks. File durable tasks explicitly when
you want to:

- Split pre-planned or multi-phase work into an epic + children
  (`file_epic_with_children`).
- Pre-queue work the user wants done in a later turn (`create_task`).
- Record a follow-up you noticed but can't fix right now.

## Task vs epic

Pick by structure, not by whether the work was planned first (plan-mode
outputs often describe a single task). The decision test runs once and
covers all three call sites:

> Could a single child close to `done` and let the user meaningfully
> inspect just that piece? If yes → epic. If no → one task.

- **`create_task`** — one coherent change, even across a few files.
  Sequential chores (edit → typecheck → test) are one task, not
  sub-steps.
- **`file_epic_with_children`** — ≥3 sub-steps that each pass the test:
  distinct phases, handoffs, or separable subsystems (e.g. schema →
  runtime → IPC → UI → docs). Each child closes on its own as it ships.
- Don't retroactively wrap a task in an epic if it turns out small —
  just finish it.

## Shaping the row

- `title`: imperative, ≤60 chars (`Fix login redirect loop`).
- `description`: what and why, terse. Put acceptance criteria inline
  as a `## Acceptance criteria` subsection (one observable criterion
  per line) — there is no separate field.
- `priority`: `medium` unless the user signalled otherwise.
- **One reviewable concern per row** — at top level AND among epic
  siblings. Two things a reviewer would accept/reject separately go in
  two rows, never one "misc" child.

# task transitions

Mark an explicit item `in_progress` when you start executing it and
`done` (via `update_task` or `complete_task`) when
you finish. Use `blocked` for items parked on user input.

**Close the row in the same turn the work actually ships.** An
`in_progress` row with finished work parked in it looks stuck to the
user. Call `complete_task` the moment the code change lands —
don't wait for a later turn.

**Pass `touched_files` when you close.** `complete_task`,
`update_task`, and `create_task` accept `touched_files: string[]`
(repo-relative paths edited for this effort) so Local History can
attribute writes to this item when several ran in parallel. Skip only
if you edited >100 files (assume-all fallback handles big sets). Filing
straight into `done`/`blocked` *without* `touched_files` opens no
effort, so attribution is impossible — pass it there too for "file and
close in one call" rows.

**Declare `impacts` for non-file outcomes.** `complete_task` accepts
`impacts: { kind, id, action? }[]` — one per cross-page outcome beyond
raw edits: a wiki page (`kind:"wiki"`), task (`"task"`), commit
(`"git_commit"`), finding (`"finding"`), or directory (`"directory"`)
you created/updated/completed/resolved. Each becomes a `page_ref`
backlink so the target lists this task as the cause without parsing the
summary body. In particular, name any wiki page you touched mid-turn.

Legitimate reasons to *stay* `in_progress` across a stop boundary:

- You have a question the user must answer before you can finish.
- The work is genuinely multi-turn and you're pausing partway through.

In either case, leave a note (`add_thread_note`) explaining what's
pending so the stop-hook nudge suppresses itself — it only fires for
items the agent didn't touch during the turn.

## Talking about items in chat

Refer to a task by its **quoted title**, never by id / "#N" / "the
last task" / "the in_progress one". Ids are internal tool-call handles;
users can't map "#14" to anything they see.

- ❌ `Shipped task #14.` / `Closing the previous one.`
- ✅ `Shipped task "Surface hidden tabs from the overflow dropdown".`

This holds everywhere you name a task in user-facing prose (fix
confirmations, summaries, commit bodies, status updates). `#N`/ids
belong only in tool-call arguments. If you slip, restate with the
title in the same turn.

## Wikilink every reference in body text

Task descriptions, acceptance criteria, effort summaries, thread
notes, and wiki pages render through the same markdown pipeline.
Anywhere you name a real entity that has a page, write it as a `[[…]]`
wikilink instead of inline code or a bare path — the renderer makes it
clickable and the `page_ref` graph records it as an outbound reference,
so the target's backlinks list this item.

Cheat sheet (every form is `[[…]]`; add `|label` to override text):

- `[[src/foo.ts]]` — file by repo-relative path (`:42` for a line)
- `[[dir:src/components]]` — directory (the `dir:` prefix is required;
  an extensionless path would otherwise parse as a wiki slug)
- `[[some-slug]]` — wiki page by slug (renderer shows its title)
- `[[abc1234]]` or `[[git:abc1234]]` — git commit by SHA
- `[[#42]]` — another task by id

Reserve inline code (`` `…` ``) for non-entities: identifiers,
snippets, command fragments, env vars. If it has a page, wikilink it.
The bare `[[path]]` form is correct in task summaries/descriptions
*and* in wiki page bodies — never write `@version` literals; freshness
is tracked in the DB (see [[oxplow-wiki-capture]]).

## Redos on a just-shipped item

When the user pushes back on work you just closed to `done`
(asks you to fix, redo, revert, or take a different approach to the
same concern), **reopen the existing item** — don't file a new one.

Flow:

1. `update_task` the item back to `in_progress` (this opens a
   fresh effort; the `done → in_progress` transition is the documented
   reopen path).
2. Do the new round of edits.
3. `complete_task` back to `done` with `touched_files` for the new
   effort.

The item row gets a second effort recording the redo, attributed
correctly. Filing a new "Fix the thing I just did" task fragments the
history and makes the Work panel lie about how many concerns the user
actually raised. A *new* concern still gets a new item — the rule is
scoped to "user rejected my last attempt at this same item."

# Dispatch mode

- **Inline**: small fixes (≤20 lines, ≤2 files, no risk). Orchestrator
  edits directly.
- **Subagent**: anything bigger or risky. Call
  `mcp__oxplow__dispatch_task({thread_id, item_id})` to get a ready
  brief; pass `prompt` to the general-purpose Agent tool. The brief
  already contains the item fields, AC, recent notes, and the
  subagent protocol preamble.

Subagents return a one-line `oxplow-result: { ok, itemId, … }`.
Record that as a work note via `add_thread_note`.
