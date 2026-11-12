---
name: tasks
description: The project's tasks — when filing a task is worth it, oxtask's commands (create, move, update, comment, link, delete), assigning and reordering, stages and buckets, epics and link types, and reading items (read, the next-task page, v_oxplow_task). Loads on the oxplow `run_command` tool with oxtask.task.* or oxplow.tasks.*, and on v_oxplow_task.
---

# Tasks

The project keeps its tasks in oxtask, a task store. How much of your work to
track as tasks is the project's choice (`track-work` or `work-tasks` says
so when it applies); with neither, file only what the person asks for or a
follow-up. oxplow records your turns and requests either way.

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

The answer names the id it minted (`tsk12`); undo deletes the task. It is
assigned to nobody. Title: imperative, ≤60 chars. One reviewable concern per
item. An epic is an item with children: file one when the change has three or
more steps a reviewer would inspect separately; otherwise file a single item.

Starting an item (`oxtask.task.move { id, stage: "in_progress" }`) assigns it
to your thread, at the end of its queue; the request you are on is linked to
it. oxplow never marks an item done. If you're tracking one, move it to
`done` when it ships.

**Stages and buckets.** oxtask's stages, in order: `backlog`, `ready`,
`in_progress`, `blocked`, `done`, `canceled` (`v_model_stage` lists any
store's: `id`, `title`, `bucket`). Every stage is in one of six buckets,
which is what oxplow acts on: `backlog` (filed, not ready yet), `todo`
(`ready`), `in_progress`, `blocked` (needs an answer), `done`, `canceled`
(decided against: keep the row, with the reasoning). Put the stage in the
stage, never in the title.

When the person rejects your last attempt at an item, reopen that item
rather than filing a new one.

## Assigning items

Who is going to work on an item is its assignee: a thread, the person, or
nobody. An unassigned `ready` item is the pool any thread may take.
`oxplow.tasks.assign { uri, to }` gives it to a thread
(`oxplow:oxplow/thread?id=thr3`) or the person (`oxplow:oxplow/person`;
`to: null`: nobody) at the end of their queue; `oxplow.tasks.reorder { uri,
before? | after? }` moves it within its queue. A controller thread routes
work this way: assign items to the threads that should do them, in the order
they should do them. A done or canceled item is nobody's (closing it
releases it, and so does closing its thread); who worked on it is its
requests (`v_oxplow_request_link`, below).

## Changing items

By the task's id (`tsk12`): `oxtask.task.move { id, stage }`,
`oxtask.task.update { id, title?, body?, priority?, parent? }`,
`oxtask.task.link { id, to, type }` (`to`: a task id or any uri) and
`oxtask.task.unlink` (the same input), `oxtask.task.comment { id, body }` (the
answer names the comment), `oxtask.task.delete { id }` (it can be restored).
Each is undoable.

Link types:

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
- `oxplow_next_task { thread: "thr3" }` — what to pick up next: the ready
  items assigned to the thread (an epic with its ready descendants, or the
  ready items), else the pool (`role: pool`), most pressing first. Every
  thread's ready queue is `v_oxplow_ready_queue` (`thread_id` NULL for the
  pool, `ord` 1 next).
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
