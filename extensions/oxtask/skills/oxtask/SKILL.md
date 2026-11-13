---
name: oxtask
description: oxtask, the project's own task tracker — when filing a task is worth it, its commands (create, move, update, comment, link, delete), its stages, epics, and the next-task page. Loads on the oxplow `run_command` tool with oxtask.task.*, and on oxtask.
---

# oxtask

The project keeps its tasks in oxtask, a task store. The `tasks` skill
teaches how to read the task model, assign and reorder items, and the link
types; this one teaches oxtask's commands. How much of your work to track
as tasks is the project's choice (`track-work` or `work-tasks` says so when
it applies); with neither, file only what the person asks for or a
follow-up.

## When an item is worth filing

File one when it helps the person follow the work:

- a plan of several separately reviewable steps (an epic and children,
  each child filed with `"parent": "<the epic's id>"`);
- work the person wants done in a later turn;
- a follow-up you noticed but can't do now.

```json
{ "id": "oxtask.task.create",
  "input": { "title": "Fix login redirect loop", "body": "…", "stage": "ready" } }
```

The answer names the id it minted (`tsk42`); undo deletes the task. It is
assigned to nobody. Title: imperative, ≤60 chars. One reviewable concern per
item. An epic is an item with children: file one when the change has three or
more steps a reviewer would inspect separately; otherwise file a single item.

Starting an item (`oxtask.task.move { id, stage: "in_progress" }`) assigns it
to your thread, at the end of its queue; the request you are on is linked to
it. oxplow never marks an item done. If you're tracking one, move it to
`done` when it ships.

**Stages.** oxtask's stages, in order: `backlog`, `ready` (bucket `todo`),
`in_progress`, `blocked`, `done`, `canceled`. The buckets are in the `tasks`
skill. Put the stage in the stage, never in the title.

When the person rejects your last attempt at an item, reopen that item
rather than filing a new one.

## Changing items

By the task's id (`tsk42`): `oxtask.task.move { id, stage }`,
`oxtask.task.update { id, title?, body?, priority?, parent? }`,
`oxtask.task.link { id, to, type }` (`to`: a task id or any uri) and
`oxtask.task.unlink` (the same input), `oxtask.task.comment { id, body }` (the
answer names the comment), `oxtask.task.delete { id }` (it can be restored).
Each is undoable. The link types are in the `tasks` skill.

## What to pick up next

`oxtask_next_task` is the tool of oxtask's `next-task` page. Give it
`{ thread: "thr3" }`: it lists the ready items assigned to the thread (an
epic with its ready descendants, or the ready items), else the pool
(`role: pool`), most pressing first. Every thread's ready queue is
`v_oxtask_ready_queue` (`thread_id` is a `v_thread` id, NULL for the pool;
`ord` 1 next). The
project's workflow (`v_oxtask_task_workflow`) says how much of this you do
unasked; set with `oxtask.tasks.set_workflow`.
