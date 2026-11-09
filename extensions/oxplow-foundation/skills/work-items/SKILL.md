---
name: work-items
description: The project's work list — when filing a work item is worth it, the work_item.* commands (create, transition, update, link, comment), states, epics and link types, and reading items (read, the work-next page, v_work_item). Loads on the oxplow `run_command` tool with work_item.*, and on v_work_item.
---

# Work items

The project has a work list (the one the person chose). Filing is
optional: oxplow tracks your work in efforts either way.

## When an item is worth filing

File one when it helps the person follow the work:

- a plan of several separately reviewable steps (an epic and children,
  each child filed with `"parent_uri": "<the epic's uri>"`);
- work the person wants done in a later turn;
- a follow-up you noticed but can't do now.

```json
{ "id": "oxplow.work_item.create",
  "input": { "title": "Fix login redirect loop", "body": "…", "state": "todo" } }
```

It goes on the active work list, assigned to nobody unless `assignee`
names a thread (`thr3`) or the person (`oxplow:oxplow/person`). `state`
is `todo` (the default), `backlog` or `in_progress`; the list's own fields go under `native`, by the names it
declares (`v_capability_provider.fields`; e.g. `{ "priority": "high" }`). Title: imperative, ≤60 chars. One reviewable
concern per item. An epic is an item with children: file one when the
change has three or more steps a reviewer would inspect separately;
otherwise file a single item.

Starting an item (`oxplow.work_item.transition { uri, to: "in_progress" }`)
assigns it to your thread and links your effort to it; starting an
unrelated one begins a new effort.
oxplow never marks an item done. If you're tracking one, move it to
`done` when it ships.

**States:** `backlog` (filed, not ready yet), `todo` (ready), `in_progress`, `blocked` (needs an
answer), `done`, `canceled` (decided against: keep the row, with the
reasoning). A list may have its own finer state (`native_state`). Put
state in the state, never in the title.

When the person rejects your last attempt at an item, reopen that item
rather than filing a new one.

## Assigning items

Who is going to work on an item is its assignee: a thread, the person, or
nobody. An unassigned `todo` item is the pool any thread may take.
`oxplow.work_item.assign { uri, to, before?, after? }` gives it to a
thread or the person (`to: null`: nobody) at the end of their queue or
next to an item there; `oxplow.work_item.reorder { uri, before? | after? }`
moves it within its queue. That queue order is core's, whatever list
holds the items. A controller thread routes work this way: assign items
to the threads that should do them, in the order they should do them.
A done or canceled item is nobody's (closing it releases it); who worked
on it is its efforts.

## Changing items

The same commands change any list's items, by canonical uri
(`oxplow:oxplow/work_item?id=<id>&provider=<provider>`): `oxplow.work_item.transition { uri, to,
native_state? }`, `oxplow.work_item.update { uri, title?, body?, parent_uri?,
state?, native? }`, `oxplow.work_item.link { uri, target, link_type }`
(and `oxplow.work_item.unlink`, the same input),
`oxplow.work_item.comment { uri, body }`. `oxplow.work_item.create` takes
`links: [{ target, link_type }]` too. The active list's own ids work as
refs too (oxplow's tasks: `tsk12`). `list_commands` shows only what the
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
  items about something with `v_work_item_link WHERE to_uri = :uri AND
  link_type = 'about'`, never by matching titles.

## Reading items

- `oxplow_read { uri }` — one item: its row, its links and its comments.
- `oxplow_work_next { thread: "thr3" }` — what to pick up next: the ready
  items assigned to the thread (an epic with its ready descendants, or the
  ready items), else the pool (`role: pool`), most pressing first.
- `oxplow_thread_work { thread }` — what is assigned to a thread and what
  its efforts finished, with their efforts.
- `oxplow_query_sql` over `v_work_item` (`uri`, `title`, `body`, `state`,
  `native_state`, `native`, `parent_uri`, `assignee` — a thread's or the
  person's uri, NULL for nobody — `rank`, `closed_at`), `v_work_item_link` and
  `v_work_item_comment` for anything else (`assignee` joins `v_thread` on its `uri`): a queue in order is
  `WHERE assignee = … ORDER BY rank IS NULL, rank, created_at`.

What's been done on an item is its efforts: `v_effort` where `work_item`
is the item's uri (`started_at`, `ended_at`, `summary`).
