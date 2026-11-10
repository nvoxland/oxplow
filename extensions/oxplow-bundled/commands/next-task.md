---
description: Pick up the next ready task and work on it.
---

Call `oxplow_next_task` (the foundation page `oxplow/next-task`) with
`thread` set to your thread's id (`thr3`). It lists what is assigned to
your thread, else the pool (`role: pool`: ready items assigned to
nobody). If it lists nothing, nothing is ready: say so and stop.

Otherwise pick the item (or, for an epic, its first ready `child`), move
it to `in_progress` with `oxplow.task.transition` — that assigns it to
your thread and links your work to it — and do it. Move it to `done` when
it ships.
