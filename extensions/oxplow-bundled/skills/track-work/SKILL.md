---
name: track-work
description: How to keep oxtask, the project's task tracker, showing what you work on — take or file a task when you start something new, move it to in progress at once, and to done only when it is fully done. Loads at the start of any work that will change files, and on the oxplow `run_command` tool with oxplow.task.transition or oxplow.task.create.
---

# Track your work as tasks

The person follows what each thread works on in its Work panel and on the
Work board. Both show tasks, and nothing else: a turn or a prompt is not on
them. Keep them true.

## When you start something new

Something new is work the person will want to see as one item: a feature,
a fix, a refactor, an investigation that ends in a change. A question
answered, a one-line fix or a command run is not: it needs no task.

1. Look for the task that already says it: the thread's own
   (`oxplow_thread_tasks { thread }`), then the open ones
   (`SELECT uri, title, state FROM v_task WHERE state NOT IN ('done','canceled')`
   through `oxplow_query_sql`, or `oxplow_search` with kind `task`).
2. None says it: file one (`oxplow.task.create`, the `tasks` skill), a
   title the person would recognize.
3. Move it to `in_progress` before you edit:

```json
{ "id": "oxplow.task.transition",
  "input": { "uri": "<the task's uri>", "to": "in_progress" } }
```

That also takes it: it's assigned to your thread, and the Work panel shows
it under "In progress" at once.

## While you work

One task in progress at a time per thread is the norm. When the work moves
to something else, move the first task back (`todo`, or `blocked` with a
comment that says why) before you start the next. Work that belongs to an
epic is its child task, not the epic.

## When it is done

Move it to `done` when it is fully done: the change is made, tests pass,
and it is committed if the project commits. Not when a step is done, and
not when you stop to ask the person something. If the person stops it,
`canceled`.

```json
{ "id": "oxplow.task.transition", "input": { "uri": "<the task's uri>", "to": "done" } }
```

oxplow links each task to the requests that worked on it from what it
observes; you never report that.
