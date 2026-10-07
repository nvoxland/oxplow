---
description: Pick up the next ready oxplow task and work on it.
---

Call `mcp__oxplow__read_task_options` for this thread. If it returns
`{ mode: "empty" }`, nothing is ready: say so and stop.

Otherwise pick the item (or, for an epic, its first ready child), move
it to `in_progress` with `work_item.transition` so your work links to
it, and do it. Move it to `done` when it ships.
