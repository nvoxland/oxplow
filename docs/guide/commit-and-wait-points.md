# Commit & wait points

Commit and wait points are inline gates in the work queue. They
share the queue's `sort_index` with work items, so you can drop
one between any two items and the agent will hit them in order.

## Commit points

A **commit point** says: "when you reach this line, run `git
commit` for me." The commit message comes from the work items
that landed since the last commit point — oxplow assembles it
automatically.

Use commit points when:

- The next chunk of work is logically independent and you want a
  clean commit boundary.
- You want a known-good state to fall back to before the agent
  starts something risky.
- The work crosses a milestone you'd want to see in `git log`.

You don't have to use them. You can run the queue without any,
or you can use auto-commit mode (below) and skip them entirely.

## Wait points

A **wait point** says: "when you reach this line, stop and wait
for me." The agent halts. The Plan pane shows the wait point as
blocked. You release it with a click and the agent resumes.

Use wait points when:

- You want a human in the loop before a risky step (database
  migration, destructive refactor, change to shared config).
- You want to review the previous chunk before the agent moves
  on.
- You want to deliberately interrupt a long sequence so you can
  redirect.

## Auto-commit mode

If you don't want to manage commit points by hand, switch the
stream to **auto-commit mode**: the agent commits on every Stop
boundary instead of at marked points. The commit message is
derived from the items closed in that turn.

Auto-commit and explicit commit points are mutually exclusive
per stream — pick one. Most users start with auto-commit and add
explicit points only when they want a more controlled history.

## Ordering matters

Because everything shares one `sort_index`, the position of a
gate is meaningful. A wait point above a work item means
"pause *before* this item starts." A commit point below an item
means "commit *after* this item finishes." Drag to reorder
visually; the agent respects it.

## What about non-stream work?

Commit and wait points are stream-scoped. Each stream has its
own queue, its own gates, its own commit history. They don't
coordinate across streams — that's the worktree isolation rule
(see [Concepts](../getting-started/concepts.md)).
