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

- assigned agent (Claude Code or Codex, fixed at thread creation)
- agent terminal (a tmux pane that survives oxplow restarts)
- set of open tabs and active tab
- work queue view
- live agent-status indicator

Switching threads restores its tab set; the agent terminals stay
alive in the background. Different threads may use different agents;
the writer/read-only role remains a separate per-stream constraint.

## Worktree isolation

Enforced by the product, not just convention: two streams never
share a working tree. Their checkouts are different directories;
their agents see different files. Stream A cannot silently
overwrite a file stream B is editing — they aren't the same file
on disk.

## Page

A **page** is anything addressable inside a tab — a file, a
diff, a task, a wiki page, a code-quality finding, a
dashboard, a settings panel, the agent terminal. Pages share
common chrome: title + status chips + collapsible **Backlinks**
panel + browser-style back/forward navigation.

The center of the window is a stack of page tabs. The rail HUD
on the left lists the available pages and links into them.

## task

A **task** is a row in the queue with a real lifecycle:

```
ready → in_progress → done
                    ↘ blocked / canceled / archived
```

tasks are durable: they survive turns, sessions, and
crashes. The agent files them before changing project files
(enforced — see [Work queue](../guide/work-queue.md)) and
closes them when the change has shipped. You can reopen anything
by flipping it back to `in_progress`. A task description is just
markdown — if a checklist of "what does done look like".

## Wiki page

The project's **wiki** is a folder of markdown files under
`.oxplow/wiki/`, indexed in SQLite. Wiki pages support
`[[wikilinks]]` for cross-references — across pages, to repo
files (`[[src/foo.ts]]`), to specific lines (`[[src/foo.ts:42]]`),
and to git commits (`[[abc1234]]`).

The agent captures non-trivial Q&A as wiki pages automatically
— codebase walkthroughs, design rationale, comparisons,
recommendations — so the durable understanding survives past
the chat reply.

## Change Analysis

A **change** is any pair of refs you want to compare — typically
a feature branch against its parent, or your working tree against
`HEAD`. The Change Analysis dashboard turns a change into a
ranked summary: *which files should I look at first, and why*.
Files are scored by a multiplicative interestingness score
(churn × complexity × tests-missing × duplication), with pivots
to drill into a slice (one extension, one directory, only the
added files), and per-function before/after metrics so you can
see exactly which functions grew, shrank, or changed shape. See
[Change Analysis](../guide/change-analysis.md).

## Effort and snapshot

Every time the agent works on a task, that's one
**effort**. An effort accumulates **file snapshots** — one per
file the agent touched, before and after. The Local History
page groups efforts under their task and lets you compare
or restore at any point.

This is how rollback works: you don't reset the whole repo, you
restore the files this effort touched.

## Backlinks

Wiki pages, tasks, files, and code-quality findings are
linked both ways. Open a task and the Backlinks panel
shows every wiki page that mentions it; open a wiki page and
you see every task or finding that points back. The rail's
recent-files and active items also surface as backlinks where
relevant.

## How they fit together

You start a **stream** on a branch. You give it a writer
**thread**. You file **tasks** describing what you want
done; the agent works through them, capturing **efforts** and
**snapshots** as it goes. Decisions and exploration land in
**wiki pages**. You navigate the project as **pages** in a
browser-like tab UI, with the rail HUD as your home base.

Repeat for as many streams as you can supervise.
