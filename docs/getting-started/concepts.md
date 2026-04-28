# Concepts

A handful of ideas to hold in your head. Everything else in the
product is built from these.

## Stream

A **stream** is one branch + one worktree. Streams are the unit
of parallelism — each stream's agent works in its own checkout,
so no two agents ever compete for the same file on disk.

There is exactly one **primary** stream — the one rooted at the
project directory itself. Every other stream is a **worktree**
stream with its own checkout next to the project root.

## Thread

A **thread** is an independent line of work inside a stream.
Each stream has at least one thread, often more — a writer
thread that ships changes, plus optional research / review
threads that ask questions without modifying files.

One thread per stream is the **writer**: it owns file edits.
Every other thread is **read-only** — its writes are denied at
the hook level. Use read-only threads for research, code
walkthroughs, or any agent task where you don't want files
changing behind your back.

Each thread carries its own:

- agent terminal (a tmux pane that survives oxplow restarts)
- set of open tabs and active tab
- work queue view
- live agent-status indicator

Switching threads restores its tab set; the agent terminals stay
alive in the background.

## Worktree isolation

Enforced by the product, not just convention: two streams never
share a working tree. Their checkouts are different directories;
their agents see different files. Stream A cannot silently
overwrite a file stream B is editing — they aren't the same file
on disk.

## Page

A **page** is anything addressable inside a tab — a file, a
diff, a work item, a wiki note, a code-quality finding, a
dashboard, a settings panel, the agent terminal. Pages share
common chrome: title + status chips + collapsible **Backlinks**
panel + browser-style back/forward navigation.

The center of the window is a stack of page tabs. The rail HUD
on the left lists the available pages and links into them.

## Work item

A **work item** is a row in the queue with a real lifecycle:

```
ready → in_progress → done
                    ↘ blocked / canceled / archived
```

Work items are durable: they survive turns, sessions, and
crashes. The agent files them before changing project files
(enforced — see [Work queue](../guide/work-queue.md)) and
closes them when acceptance criteria are met. You can reopen
anything by flipping it back to `in_progress`.

## Wiki note

The project's **wiki** is a folder of markdown files under
`.oxplow/notes/`, indexed in SQLite. Notes support
`[[wikilinks]]` for cross-references — across notes, to repo
files (`[[src/foo.ts]]`), and to git commits (`[[abc1234]]`).

The agent captures non-trivial Q&A here automatically — codebase
walkthroughs, design rationale, comparisons, recommendations —
so the durable understanding survives past the chat reply.

## Effort and snapshot

Every time the agent works on a work item, that's one
**effort**. An effort accumulates **file snapshots** — one per
file the agent touched, before and after. The Local History
page groups efforts under their work item and lets you compare
or restore at any point.

This is how rollback works: you don't reset the whole repo, you
restore the files this effort touched.

## Backlinks

Notes, work items, files, and code-quality findings are linked
both ways. Open a work item and the Backlinks panel shows every
note that mentions it; open a note and you see every work item
or finding that points back. The rail's recent-files and active
items also surface as backlinks where relevant.

## How they fit together

You start a **stream** on a branch. You give it a writer
**thread**. You file **work items** describing what you want
done; the agent works through them, capturing **efforts** and
**snapshots** as it goes. Decisions and exploration land in
**wiki notes**. You navigate the project as **pages** in a
browser-like tab UI, with the rail HUD as your home base.

Repeat for as many streams as you can supervise.
