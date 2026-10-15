---
description: Pick up the next ready work item and work on it.
---

Run the oxplow lens `oxplow-bundled/work-next` (`oxplow_run_lens`) with
`thread` set to your thread's ref (`thread:thr3`). If it lists nothing,
nothing is ready: say so and stop.

Otherwise pick the item (or, for an epic, its first ready `child`), move
it to `in_progress` with `oxplow.work_item.transition` so your work links to
it, and do it. Move it to `done` when it ships.
