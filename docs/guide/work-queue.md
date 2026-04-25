# Work queue

The work queue is the durable, ordered list of tasks for a stream.
Each row is a **work item**. Work items survive turn boundaries,
restarts, and crashes — they are the difference between "what the
agent is doing" and "what we're trying to ship."

## Lifecycle

```
ready → in_progress → human_check → done
                    ↘ blocked
                    ↘ canceled / archived
```

- **ready** — filed but not yet picked up.
- **in_progress** — the agent (or you) is actively working it.
- **human_check** — the agent thinks it's done; waiting for your
  review.
- **blocked** — paused, needs your input or an external answer.
- **done** — *only you* set this. The agent never marks its own
  work `done`. That separation is intentional: review is the
  gate, not the agent's confidence.
- **canceled / archived** — drops out of the active view.

## When the agent files a work item

The agent files an item before it starts changing project files
in any non-trivial way. Two shapes:

- **Task** — one coherent change, even if it touches a few files.
  Most work is tasks.
- **Epic with children** — work that has three or more sub-steps a
  reviewer would naturally check off independently.

You can also file work items yourself, from the Plan pane, and
drag them into order.

## Sort order

Items, commit points, and wait points all share a single
`sort_index`. That means you can drag a wait point *between* two
work items in the queue, and the agent will hit it in that exact
order. The queue is one ordered timeline, not three parallel
ones.

## Grouping by epic

Children of an epic render under the epic header. Closing each
child to `human_check` as it ships shows progress accumulating
without finishing the epic itself. The user closes the parent
once they've reviewed the children.

## One concern per row

A row should be something a reviewer can accept or push back on
independently. If two requests would be QA'd separately ("show
the placeholder" + "drop the elapsed counter"), they get two
rows, even when the agent intends to land them in one diff.

## Reopening

When you push back on a `human_check` row — "no, redo this" —
the row flips back to `in_progress`. A new effort opens against
it, attributed to the redo, so Local History can tell you what
the second attempt actually changed.

This is the main reason oxplow does *not* file new tasks for
fixes: a redo of the same concern stays on the same row, with
two efforts inside it. Otherwise the queue fills with noise.

## Where to drive it

- **Plan pane** — drag, drop, reorder, file, edit, archive.
- **MCP tools** — see [MCP tools](../reference/mcp-tools.md) for
  the surface the agent uses (`create_work_item`,
  `update_work_item`, `complete_task`, `add_work_note`,
  `file_epic_with_children`).
- **Keyboard** — see [Keybindings](../reference/keybindings.md).
