---
name: work-tasks
description: How to work the tasks the person files in oxtask, the project's task tracker — work what is assigned or queued, move a task to in progress before you edit and to done only when it is fully done, and file a task only for a follow-up or when asked. Loads at the start of any work that will change files, and on the oxplow `run_command` tool with oxplow.task.transition or oxplow.task.create.
---

# Work the tasks the person files

In this project the person files the tasks and the agents work them. The
Work panel and the Work board show what each thread works on; keep them
true for the tasks you work.

## What to work

- What the person asks you directly is the work: do it. Don't file a task
  for it.
- When they ask you to work the queue, take the next task (the `next-task`
  command): the thread's own first, then the ready ones.
- A task is one item: the harness's own plan or todo list is for the
  steps inside it.

## While you work a task

1. Move it to `in_progress` before you edit (`oxplow.task.transition`,
   `to: in_progress`). That also assigns it to your thread.
2. One task in progress at a time per thread. When you stop on it, move it
   back (`todo`, or `blocked` with a comment that says why).
3. Work you find that the task does not cover: file it as a follow-up on
   the backlog (`oxplow.task.create`, the `tasks` skill), linked
   `discovered_from` the task. Don't do it unasked.

## When it is done

Move it to `done` only when it is fully done: the change is made, tests
pass, it is committed if the project commits, and the project's definition
of done holds when it has one. Not when a step is done, and not when you
stop to ask the person something.

oxplow links each task to the requests that worked on it from what it
observes; you never report that.
