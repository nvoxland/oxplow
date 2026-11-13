---
name: tasks
description: Reading the project's tasks, whichever store keeps them — the task model (v_oxplow_task, v_oxplow_task_link, v_oxplow_task_comment, v_oxplow_request_link, v_model_stage), buckets and stages, link types, assigning and reordering, and which requests worked on an item. Loads on the oxplow `run_command` tool with oxplow.tasks.*, and on v_oxplow_task.
---

# Tasks

The project's tasks are rows of one shared model (`oxplow/task_record`),
filled by a task store. The store's own skill teaches its commands (find it
in the skills list); this one teaches how to read the model. How much of
your work to track as tasks is the project's choice (a workflow skill says
so when it applies); with none, file only what the person asks for or a
follow-up. oxplow records your turns and requests either way.

## Stages and buckets

A store has its own stages (`v_model_stage` lists any store's: `model`,
`store`, `id`, `title`, `bucket`). Every stage is in one of six buckets,
which is what oxplow acts on: `backlog` (filed, not ready yet), `todo`
(ready), `in_progress`, `blocked` (needs an answer), `done`, `canceled`
(decided against: keep the row, with the reasoning). Put the stage in the
stage, never in the title.

Changing a task is the store's job: its commands act through the task action
points (`oxplow/task.new`, `task.move`, `task.edit`, `task.unparent`,
`task.delete`, `task.comment`, `task.link`, `task.unlink`), which every
store answers for its own tasks. oxplow never marks an item done.

## Assigning items

Who is going to work on an item is its assignee: a thread, the person, or
nobody. An unassigned item in the `todo` bucket is the pool any thread may
take. `oxplow.tasks.assign { uri, to }` gives it to a thread
(`oxplow:oxplow/thread?id=thr3`) or the person (`oxplow:oxplow/person`;
`to: null`: nobody) at the end of their queue; `oxplow.tasks.reorder { uri,
before? | after? }` moves it within its queue. A controller thread routes
work this way: assign items to the threads that should do them, in the order
they should do them. A done or canceled item is nobody's (closing it
releases it, and so does closing its thread); who worked on it is its
requests (`v_oxplow_request_link`, below).

## Link types

- **blocks**: the item must finish before the target can start (a
  migration before the feature that uses it).
- **discovered_from**: the item was uncovered while working on the
  target. File scope creep separately and link it back.
- **relates_to**: a general association with no ordering.
- **duplicates**: the same work as the target; close the duplicate.
- **supersedes**: replaces the target, which is stale.
- **replies_to**: a threaded response to the target.
- **about**: what the item is about — any uri, not only another item
  (`oxplow:oxplow/request?id=req12`, `oxplow:oxplow/file?path=src/a.rs`, `oxplow:oxplow/collector?collector=triage/hot`). Find the
  items about something with `v_oxplow_task_link WHERE to_uri = :uri AND
  link_type = 'about'`, never by matching titles.

## Reading items

- `oxplow_read { uri }` — one item: its row, its links and its comments.
- `oxplow_thread_tasks { thread }` — what is assigned to a thread and what
  it finished, with the requests that worked on each.
- `oxplow_query_sql` over `v_oxplow_task` (`uri`, `title`, `body`, `state` —
  the bucket — `stage`, `stage_title`, `native`, `parent_uri`, `assignee` — a
  thread's or the person's uri, NULL for nobody — `rank`, `closed_at`),
  `v_oxplow_task_link` and `v_oxplow_task_comment` for anything else
  (`assignee` joins `v_thread` on its `uri`): a queue in order is
  `WHERE assignee = … ORDER BY rank IS NULL, rank, created_at`.

What's been done on an item is its requests: `v_oxplow_request_link` where
`task` is the item's uri says which requests worked on it and on what
evidence (`basis`: `prompt` — a person's prompt named it; `filed` or
`edited` — the agent filed, updated or commented on it; `transition` — the
agent moved it between stages; `assigned` — it was in progress and assigned
to the thread when a turn ended, the weakest), with the request's turn count
and times. Join `request_id` to `v_request` (its state, prompt, tokens) and
`v_commit_request` (the commits it made); a thread's tasks by request are
`WHERE thread_id = …`. A request's turns are `v_agent_turn WHERE request_id =
…`, and the files they changed `v_turn_file` over those turns (`confidence`
`claimed` or `sole` is the turn's own; `shared` another turn could have made).
