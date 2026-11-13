---
description: Pick up the next ready task and work on it.
---

Call `oxtask_next_task` (the page `oxtask/next-task`) with `thread` set
to your thread's id (`thr3`). It lists what is assigned to your thread,
else the pool (`role: pool`: ready items assigned to nobody), from
`v_oxtask_ready_queue` (every thread's ready queue, `thread_id` NULL for
the pool, `ord` 1 next). If it lists nothing, nothing is ready: say so
and stop.

Otherwise pick the item (or, for an epic, its first ready `child`), move
it to `in_progress` with `oxtask.task.move { id, stage: in_progress }` — that assigns it to
your thread and links your work to it — and do it. Move it to `done` when
it ships.
