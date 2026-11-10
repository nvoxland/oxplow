# Tasks

Tasks keep plans and follow-ups durable. They can describe a coherent
change, or group independently reviewable steps under an epic. Oxplow can
use oxtask, an external task tracker, or no task tracker at all.

Work tracking does not require filing an item before an edit. Turns,
tool calls, snapshots, and test evidence are recorded independently.
Use tasks when they help people and agents follow the work.

## Lifecycle

Every task tracker declares its own stages, in its own order and with its
own names. Each stage is in one of six buckets, and those are what oxplow
acts on:

```text
backlog → todo → in_progress → done
                             ↘ blocked / canceled
```

- **backlog** — filed, but not ready to work on yet.
- **todo** — ready. An idle thread can pick up an unassigned one.
- **in_progress** — work someone is doing.
- **blocked** — work waiting on an answer or another dependency.
- **done** — finished work.
- **canceled** — work that will not be completed.

The built-in list's stages are Backlog, Ready, In Progress, Blocked,
Done, and Canceled, one per bucket. Another list can have several stages
in one bucket, such as "In Review" next to "In Progress". The board, the
Task List's State filter and its Mark menu show the list's stages. A
command that takes a stage also takes a bucket name, and lands on the
first stage the list declares in that bucket.

Oxplow does not automatically mark work done. A person or agent updates
the item. The selected effort policy can link activity to an item and
move linked to-do work into progress; see
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

The built-in task tracker provides thread lists and a backlog, with grouping,
ordering, comments, and links. Open work pages from the launcher; use
their row actions to create, edit, move, assign, or cancel items. An external
provider's available actions depend on its declared capabilities.

Agents read work through the semantic layer and change it through the
same audited commands as the UI. Starting an item does not send a prompt
to a coding agent or force it to continue after it stops.

## Tasks and efforts

An effort is a span of a thread's activity, optionally linked to a task. One item can have several efforts; an effort needs no item. The
effort policy determines grouping, while core records attribution and
evidence. Planning and recording therefore remain useful independently
of one another.
