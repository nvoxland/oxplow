# Concepts

Six ideas to hold in your head. Everything else in the product is
built from these.

## Stream

A **stream** is one branch + one worktree + one agent + one work
queue. Streams are the unit of parallelism. You add a stream when
you want a new piece of work that runs alongside whatever is
already in flight, on its own branch, without colliding with the
other streams' files.

There is exactly one **primary** stream — the one rooted at the
project directory itself. Every other stream is a **worktree**
stream with its own checkout under `.oxplow/worktrees/<slug>/`.

## Thread

A **thread** is an agent conversation inside a stream. Each
stream has at least one thread. Most streams have one. You can
add more.

One thread per stream is the **writer**: it owns file edits.
Every other thread is **read-only** — it can browse, search, run
the agent, ask questions, but its writes are denied at the hook
level. Use read-only threads when you want a research or
review-style agent that won't modify files behind your back.

## Worktree isolation

This one is enforced by the product, not just convention: two
streams never share a working tree. Their checkouts are different
directories. Their agents see different files. There is no way
for stream A to silently overwrite a file stream B is editing,
because they are not the same file on disk.

## Work item

A **work item** is a row in the queue with a real lifecycle:

```
ready → in_progress → human_check → done
                    ↘ blocked
                    ↘ canceled
```

You file work items when you want a durable record of intent —
the thing you want to ship, not the conversation about it. Work
items survive across turns, sessions, and crashes.

The agent files them too, via MCP. When the agent realizes it's
about to change project files, it files a work item and tracks
its progress against the row.

## Commit point and wait point

Inline markers in the work queue, ordered by `sort_index` (shared
with work items, so they intermix):

- **Commit point.** When the agent reaches it, oxplow runs `git
  commit` automatically. Use these to mark "this much work
  belongs together as one commit."
- **Wait point.** When the agent reaches it, oxplow blocks the
  agent until you release the gate. Use these to force a human
  check before the agent moves on.

Auto-commit mode is the alternative for low-friction loops: the
agent commits on every Stop instead of at marked points.

## Effort and snapshot

Every time the agent runs against a work item, that's one
**effort**. An effort accumulates **file snapshots** — one per
file the agent touched, before and after. The Local History
modal groups efforts under their work item and lets you compare
or restore at any point.

This is how rollback works: you don't reset the whole repo, you
restore the files this effort touched.

## How they fit together

You start a **stream** on a branch. You give it a writer
**thread**. You file **work items** describing what you want
done, optionally interleaved with **commit points** and **wait
points**. The agent works through the queue, producing
**efforts** and **snapshots** as it goes. Anything you don't like
gets pushed back via the work item's lifecycle, restored from
Local History, or both.

Repeat for as many streams as you can supervise.
