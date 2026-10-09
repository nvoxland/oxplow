# Work queue

Work items keep plans and follow-ups durable. They can describe a coherent
change, or group independently reviewable steps under an epic. Oxplow can
use its built-in task list, an external work-item provider, or no work
list at all.

Work tracking does not require filing an item before an edit. Turns,
tool calls, snapshots, and test evidence are recorded independently.
Use work items when they help people and agents follow the work.

## Lifecycle

Every provider maps its states to a common vocabulary:

```text
todo → in_progress → done
                   ↘ blocked / canceled
```

The built-in list displays `todo` as **To Do** and can archive items.
Another provider may have additional native states or fields.

- **To Do** — planned work that has not started.
- **In Progress** — work someone is doing.
- **Blocked** — work waiting on an answer or another dependency.
- **Done** — finished work.
- **Canceled** — work that will not be completed.

Oxplow does not automatically mark work done. A person or agent updates
the item. The selected effort policy can link activity to an item and
move linked To Do work into progress; see
[Local History](local-history.md#effort).

## Planning useful items

Give each item an outcome a reviewer can assess independently. Its
markdown description can include scope, acceptance criteria, evidence,
and dependencies. Use an epic when the work has several separately
reviewable steps.

Reopen an existing item when correcting the same outcome. File a new
item for a new concern or a follow-up you will not do now. Link
prerequisites rather than hiding them in prose alone.

## Lists and actions

The built-in work list provides thread lists and a backlog, with grouping,
ordering, comments, and links. Open work pages from the launcher; use
their row actions to create, edit, move, or archive items. An external
provider's available actions depend on its declared capabilities.

Agents read work through the semantic layer and change it through the
same audited commands as the UI. Starting an item does not send a prompt
to a coding agent or force it to continue after it stops.

## Work items and efforts

An effort is a span of a thread's activity, optionally linked to a work
item. One item can have several efforts; an effort needs no item. The
effort policy determines grouping, while core records attribution and
evidence. Planning and recording therefore remain useful independently
of one another.
