# Work queue

The work queue is the durable, ordered list of tasks for a thread.
Each row is a **work item**. Work items survive turn boundaries,
restarts, and crashes — they are the difference between "what the
agent is doing right now" and "what we're trying to ship."

## Lifecycle

```
ready → in_progress → done
                    ↘ blocked
                    ↘ canceled / archived
```

- **ready** — filed but not yet picked up. Backlog.
- **in_progress** — the agent (or you) is actively working it.
- **blocked** — paused; needs your input or an external answer.
- **done** — acceptance criteria met. The agent closes its own
  items via `complete_task`; you can reopen anything by flipping
  it back to `in_progress`.
- **canceled** — abandoned without shipping.
- **archived** — drops out of the default views.

`ready` vs `in_progress` is **not** "noticed it" vs "started it" —
it's "backlog" vs "committed to ship now". The agent's filing
guard cares about the difference: it can't edit project files
unless an item is `in_progress`.

## Where to find work

The rail HUD on the left of every window surfaces the live state
of the queue:

- **Active item** — the lowest-priority `in_progress` item for
  the current thread, with a live agent-status dot.
- **Up next** — the top five `ready` items.
- A **Backlog** entry under "Pages" with a count badge for the
  stream-global backlog.

Full lists live in dedicated pages, opened from the rail's
**Pages** directory:

- **Plan work** — the current thread's planning surface (To Do +
  Blocked, with previews of recently done).
- **Done work** — full descending list of done + canceled items.
- **Backlog** — the stream-global backlog.
- **Archived** — archived items only.

## When the agent files an item

The agent files an item before it starts changing project files.
Two shapes:

- **Task** — one coherent change, even if it touches a few files.
  Most work is tasks. (`create_work_item`, kind defaults to
  `task`.)
- **Epic with children** — work that has three or more sub-steps a
  reviewer would naturally check off independently.
  (`file_epic_with_children`.)

A `PreToolUse` hook on the writer thread denies `Edit` / `Write` /
`MultiEdit` / `NotebookEdit` if the thread has no `in_progress`
item — so the filing-before-editing rule is mechanical, not a
convention.

## One concern per row

A row should be something a reviewer can accept or push back on
independently. If two requests would be QA'd separately ("show
the placeholder" + "drop the elapsed counter"), they get two
rows, even when the agent intends to land them in one diff.

When you send a new prompt mid-flight while an item is still
in progress, the runtime injects a reminder: file a new row, or
explicitly reopen the existing one if this is a fix to the same
concern. Multi-prompt turns shouldn't quietly pile new asks into
whichever item happened to be open.

## Reopening vs filing fresh

When you push back on a `done` row — "no, redo this" — the
correct move is to flip it back to `in_progress`, redo the work,
and `complete_task` again. A new effort opens against the same
item, attributed to the redo.

This is why oxplow does *not* file new tasks for fixes: a redo of
the same concern stays on the same row, with multiple efforts
inside it. Otherwise the queue fills with "Fix what I just did"
noise.

Genuinely new asks still get new items.

## Sort order and grouping

`sort_index` is a single ordered field per thread. Drag a row to
reorder; the agent's queue follows. Children of an epic render
under the epic header.

## Where to drive it

- **Plan work / Backlog pages** — drag, drop, reorder, file,
  edit, archive. Multi-select for bulk transitions.
- **Rail HUD** — quick navigation; never auto-opens tabs.
- **Inline new-row** at the top of any work list — type a title,
  press Enter.
- **MCP tools** — `create_work_item`, `update_work_item`,
  `complete_task`, `add_work_note`, `file_epic_with_children`,
  `transition_work_items`. See [MCP tools](../reference/mcp-tools.md).

## Followups

The agent can stash transient sub-asks ("come back to this in
this same turn") via `add_followup` / `remove_followup`. They
aren't durable rows — they live in memory, survive only until the
runtime restarts, and surface as italic muted "↳ follow-up: …"
lines at the top of To Do. Use them to defer something mid-turn
without polluting the work queue.
