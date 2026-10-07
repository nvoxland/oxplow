---
name: work-items
description: The project's work list — when filing a work item is worth it, the work_item.* commands (create, transition, update, link, comment), states, epics and link types, and reading items (list_work_items, get_work_item, next_work_item, v_work_item). Loads on mcp__oxplow__run_command with work_item.*, on the work-item tools, and on v_work_item.
---

# Work items

The project has a work list (the one the person chose). Filing is
optional: oxplow tracks your work in efforts either way.

## When an item is worth filing

File one when it helps the person follow the work:

- a plan of several separately reviewable steps (an epic and children,
  each child filed with `"parent_ref": "<the epic's ref>"`);
- work the person wants done in a later turn;
- a follow-up you noticed but can't do now.

```json
{ "name": "work_item.create",
  "input": { "title": "Fix login redirect loop", "body": "…", "state": "todo" } }
```

It goes on the active work list. `state` is `todo` (the default) or
`in_progress`; the list's own fields go under `native`, by the names it
declares (`v_capability_provider.fields`; e.g. `{ "priority": "high" }`). Title: imperative, ≤60 chars. One reviewable
concern per item. An epic is an item with children: file one when the
change has three or more steps a reviewer would inspect separately;
otherwise file a single item.

Starting an item (`work_item.transition { ref, to: "in_progress" }`)
links your effort to it; starting an unrelated one begins a new effort.
oxplow never marks an item done. If you're tracking one, move it to
`done` when it ships.

**States:** `todo` (ready), `in_progress`, `blocked` (needs an
answer), `done`, `canceled` (decided against: keep the row, with the
reasoning). A list may have its own finer state (`native_state`). Put
state in the state, never in the title.

When the person rejects your last attempt at an item, reopen that item
rather than filing a new one.

## Changing items

The same commands change any list's items, by canonical ref
(`work_item:<provider>:<id>`): `work_item.transition { ref, to,
native_state? }`, `work_item.update { ref, title?, body?, parent_ref?,
state?, native? }`, `work_item.link { ref, target, link_type }`,
`work_item.comment { ref, body }`. The active list's own ids work as
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

## Reading items

- `list_work_items { list: "thr3" | "backlog", states? }` — a list's
  items in order (open ones unless `states` says).
- `get_work_item { ref }` — one item with its links and comments.
- `next_work_item { thread_id }` — what to pick up next (an epic with its
  ready children, or the ready items).

For anything else, `query_sql` over `v_work_item` (`ref`, `title`,
`body`, `state`, `native_state`, `native`, `parent_ref`, `thread_id` —
NULL on the backlog — `rank`, `closed_at`), `v_work_item_link` and
`v_work_item_comment`. What's been done on an item is its efforts:
`v_effort` where `work_item` is the item's ref (`started_at`,
`ended_at`, `summary`).
