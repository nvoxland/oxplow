---
name: tasks
description: The project's task tracker — when filing a task is worth it, the task.* commands (create, transition, update, link, comment), stages and buckets, epics and link types, and reading items (read, the next-task page, v_task). Loads on the oxplow `run_command` tool with task.*, and on v_task.
---

# Tasks

The project has a task tracker (the one the person chose). How much of
your work to track as tasks is that tracker's opinion: its own skill says
so when it has one. oxplow records your turns and requests either way.

## When an item is worth filing

File one when it helps the person follow the work:

- a plan of several separately reviewable steps (an epic and children,
  each child filed with `"parent_uri": "<the epic's uri>"`);
- work the person wants done in a later turn;
- a follow-up you noticed but can't do now.

```json
{ "id": "oxplow.task.create",
  "input": { "title": "Fix login redirect loop", "body": "…", "stage": "todo" } }
```

It goes on the active task tracker, assigned to nobody unless `assignee`
names a thread (`thr3`) or the person (`oxplow:oxplow/person`). `stage`
is one of the list's stages or a bucket (`todo`, the default, `backlog` or
`in_progress`); the list's own fields go under `native`, by the names it
declares (`v_capability_provider.fields`; e.g. `{ "priority": "high" }`). Title: imperative, ≤60 chars. One reviewable
concern per item. An epic is an item with children: file one when the
change has three or more steps a reviewer would inspect separately;
otherwise file a single item.

Starting an item (`oxplow.task.transition { uri, to: "in_progress" }`)
assigns it to your thread and links your effort to it; starting an
unrelated one begins a new effort.
oxplow never marks an item done. If you're tracking one, move it to
`done` when it ships.

**Stages and buckets.** Each list declares its own stages, in order
(`v_capability_provider.stages`: `id`, `title`, `bucket`; oxtask:
`backlog`, `ready`, `in_progress`, `blocked`, `done`, `canceled`). Every
stage is in one of six buckets, which is what oxplow acts on:
`backlog` (filed, not ready yet), `todo` (ready), `in_progress`,
`blocked` (needs an answer), `done`, `canceled` (decided against: keep
the row, with the reasoning). Where a command takes a stage (`to`,
`stage`) you may name a bucket instead: it lands on the first stage the
list declares in it. Put the stage in the stage, never in the title.

When the person rejects your last attempt at an item, reopen that item
rather than filing a new one.

## Assigning items

Who is going to work on an item is its assignee: a thread, the person, or
nobody. An unassigned `todo` item is the pool any thread may take.
`oxplow.task.assign { uri, to, before?, after? }` gives it to a
thread or the person (`to: null`: nobody) at the end of their queue or
next to an item there; `oxplow.task.reorder { uri, before? | after? }`
moves it within its queue. That queue order is core's, whatever list
holds the items. A controller thread routes work this way: assign items
to the threads that should do them, in the order they should do them.
A done or canceled item is nobody's (closing it releases it); who worked
on it is its efforts.

## Changing items

The same commands change any list's items, by canonical uri
(`oxplow:oxplow/task?id=<id>&provider=<provider>`): `oxplow.task.transition { uri, to }`
(`to`: a stage or a bucket), `oxplow.task.update { uri, title?, body?, parent_uri?,
stage?, native? }`, `oxplow.task.link { uri, target, link_type }`
(and `oxplow.task.unlink`, the same input),
`oxplow.task.comment { uri, body }`. `oxplow.task.create` takes
`links: [{ target, link_type }]` too. The active list's own ids work as
refs too (oxtask: `tsk12`). `list_commands` shows only what the
active list supports.

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
  (`oxplow:oxplow/effort?id=eff12`, `oxplow:oxplow/file?path=src/a.rs`, `oxplow:oxplow/collector?collector=triage/hot`). Find the
  items about something with `v_task_link WHERE to_uri = :uri AND
  link_type = 'about'`, never by matching titles.

## Reading items

- `oxplow_read { uri }` — one item: its row, its links and its comments.
- `oxplow_next_task { thread: "thr3" }` — what to pick up next: the ready
  items assigned to the thread (an epic with its ready descendants, or the
  ready items), else the pool (`role: pool`), most pressing first.
- `oxplow_thread_tasks { thread }` — what is assigned to a thread and what
  its efforts finished, with their efforts.
- `oxplow_query_sql` over `v_task` (`uri`, `title`, `body`, `state` — the
  bucket — `stage`, `stage_title`, `native`, `parent_uri`, `assignee` — a thread's or the
  person's uri, NULL for nobody — `rank`, `closed_at`), `v_task_link` and
  `v_task_comment` for anything else (`assignee` joins `v_thread` on its `uri`): a queue in order is
  `WHERE assignee = … ORDER BY rank IS NULL, rank, created_at`.

What's been done on an item is its efforts: `v_effort` where `task`
is the item's uri (`started_at`, `ended_at`, `summary`). The files each
effort changed are `v_effort_file` (`effort_id` is `v_effort.id`).
