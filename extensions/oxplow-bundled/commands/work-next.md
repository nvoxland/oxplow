---
description: Pick up the next ready work item and work on it.
---

Call `mcp__oxplow__next_work_item` for this thread. If it returns
`{ mode: "empty" }`, nothing is ready: say so and stop.

Otherwise pick the item (or, for an epic, its first ready child), move
it to `in_progress` with `oxplow.work_item.transition` so your work links to
it, and do it. Move it to `done` when it ships.
