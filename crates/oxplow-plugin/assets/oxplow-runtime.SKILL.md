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

Pick by structure, not by whether the work was planned first. Plenty
of plan-mode outputs describe a single task.

- **`create_task`** — one coherent change, even if it touches a few
  files. Rename, bug fix, small feature in one subsystem. Sequential
  chores (edit → typecheck → test) are still one task, not sub-steps.
- **`file_epic_with_children`** — ≥3 sub-steps a reviewer would
  naturally check off independently: distinct phases, clear handoffs,
  or separable subsystems (e.g. schema → runtime → IPC → UI → docs).
  Each child closes to `done` on its own as it ships.
- Decision test: could a single child close to `done` and
  have the user meaningfully inspect just that piece? If yes, epic.
  If no, it's a task and the bullets are just an execution outline.
- Don't retroactively wrap a task in an epic mid-execution if it turns
  out to be small — just finish it.

## Shaping the row

- `title`: imperative, ≤60 chars (`Fix login redirect loop`).
- `description`: what and why; keep it terse. Put acceptance
  criteria inline as a `## Acceptance criteria` subsection (one
  observable criterion per line) — there is no separate field.
- `priority`: `medium` unless the user signalled otherwise.
- Use `file_epic_with_children` (not `create_task`) when the work
  should land as a parent + children — an "epic" is just any task
  that ends up with children.

## One QA-separate concern per row

Siblings under an epic still need to be independently reviewable:
two things a reviewer would accept/reject separately go in two child
tasks, not one "misc" child. Same rule as top-level items.

# task transitions

Mark an explicit item `in_progress` when you start executing it and
`done` (via `update_task` or `complete_task`) when
you finish. Use `blocked` for items parked on user input.

**Close the row in the same turn the work actually ships.** An
`in_progress` row with finished work parked in it looks stuck to the
user. Call `complete_task` the moment the code change lands —
don't wait for a later turn.

**Pass `touched_files` when you close.** `complete_task`,
`update_task`, and `create_task` all accept an optional
`touched_files: string[]` of repo-relative paths you edited for this
effort. The runtime attaches them to the closing effort so Local
History can attribute writes to this specific item when multiple
items ran in parallel. Skip only if you edited >100 files (the
assume-all fallback handles big change sets).

**Declare `impacts` for non-file outcomes.** `complete_task` also
accepts `impacts: { kind, id, action? }[]` — one row per
cross-page outcome of this effort beyond raw file edits. Use it
to record:

- A wiki page you created/updated/deleted (`kind: "wiki", id:
  "<slug>"`)
- Another task you completed, blocked, or reopened
  (`kind: "task", id: "<id>"`)
- A commit you referenced or rolled back
  (`kind: "git_commit", id: "<sha>"`)
- A finding you resolved (`kind: "finding", id: "<id>"`)
- A directory you reorganized (`kind: "directory", id: "<path>"`)

The runtime projects each row into the unified `page_ref` graph
under `ref_type=impact` with the `action` carried in
`source_extra`, so the target page's backlinks list this task as
the cause — without anyone parsing the summary body to find it.
The wiki-capture skill leans on this: when you file or update a
wiki page mid-turn, name it in `impacts` so the page's
"referenced by" list points back here.

For retroactive splits or "file and close in one call" rows (where
the edits already shipped and you just want a durable row with
attribution), pass `touched_files` directly into `create_task`
along with `status: "done"` (or `"blocked"`) — the server
synthesizes the `in_progress → target` transition so attribution
lands exactly as it would for a normal close. Without
`touched_files`, items filed directly into `done` never open
an effort, so attribution is impossible; the Local History panel
falls back to "assume all" for that item.

Legitimate reasons to *stay* `in_progress` across a stop boundary:

- You have a question the user must answer before you can finish.
- The work is genuinely multi-turn and you're pausing partway through.

In either case, leave a note (`add_thread_note`) explaining what's
pending so the stop-hook nudge suppresses itself — it only fires for
items the agent didn't touch during the turn.

## Talking about items in chat

When you mention a task to the user, refer to it by its **quoted
title** — not by id, not by "#N", not by "the last task", not by
"the in_progress one". The id is an internal handle for tool
calls; users don't see ids in their UI and can't map "#14" to
anything they recognize.

- ❌ `Shipped task #14.`
- ❌ `Task 14 is now in_progress.`
- ❌ `Closing the previous one.`
- ✅ `Shipped task "Surface hidden tabs from the overflow dropdown".`
- ✅ `Task "Fix login redirect loop" is now in_progress.`
- ✅ `Closing task "Render wiki page links by title".`

This rule applies everywhere you reference a task in user-facing
prose: confirming a fix, asking whether to proceed, summarizing
what shipped, naming the item you just reopened, commit body
prose, status updates, follow-up prompts. The only place `#N` /
ids are appropriate is in tool-call arguments and code identifiers
— never in the conversation surface.

If you slip and use `#N`, restate with the title in the same turn;
don't leave a half-anonymous reference for the user to decode.

## Wikilink every reference in body text

Task descriptions, acceptance criteria, effort summaries, thread
notes, and wiki pages all render through the same markdown
pipeline. Anywhere you name a real entity that has a page, write
it as a `[[…]]` wikilink instead of inline code or a bare path —
the renderer turns wikilinks into clickable, icon-bearing links
with proper navigation, and the unified `page_ref` graph picks
them up as outbound references so the target's backlinks list
this item without needing summary-body parsing.

Cheat sheet (every form is `[[…]]`; supply `|label` to override
display text):

- `[[src/foo.ts]]` — file by repo-relative path
- `[[src/foo.ts:42]]` — file + line
- `[[src/foo.ts@HEAD]]` / `[[src/foo.ts@<sha>]]` / `[[src/foo.ts@disk]]`
  — pin a version (required inside wiki page bodies; optional
  elsewhere)
- `[[dir:src/components]]` — directory (the `dir:` prefix is what
  distinguishes a directory from a file — without it, a path
  with no extension would be parsed as a wiki slug)
- `[[some-slug]]` — wiki page by slug; renderer displays the
  page's title, not the slug
- `[[abc1234]]` or `[[git:abc1234]]` — git commit by SHA
- `[[#42]]` — another task by id (when supported by the renderer)

What this replaces:

- ❌ `` `.context/data-model.md` `` (inline code — not clickable,
  not in the graph)
- ❌ ``See `src/foo.ts` for the helper`` (same problem)
- ✅ `[[.context/data-model.md]]`
- ✅ `See [[src/foo.ts|the foo helper]] for the helper.`

Reserve inline code (`` `…` ``) for things that aren't real
entities: identifiers, snippets, command fragments, env vars.
If it has a page in oxplow, it deserves a wikilink.

The [[oxplow-wiki-capture]] skill has the full version-pinning
rules for file wikilinks inside wiki page bodies — those are
stricter (every file wikilink must declare `@version`). In task
summaries and descriptions the bare `[[path]]` form is fine; the
renderer falls back to working-tree.

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
