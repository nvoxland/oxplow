# Your first stream

This page walks through the first five minutes: open a project,
send a prompt, watch what oxplow does, accept the work.

## 1. Open the project

**File → Open Project** and pick a git repo. Oxplow scopes itself
to that directory — it will not climb upward looking for an
enclosing repo. Whatever you opened *is* the workspace.

The first time you open a project, oxplow creates `.oxplow/`
inside it:

```
.oxplow/
  state.sqlite           # work items, threads, snapshots, settings
  notes/                 # wiki markdown files (`<slug>.md`)
  worktrees/             # checkouts for non-primary streams (siblings of the repo)
  runtime/               # the Claude Code plugin oxplow installs per project
```

The first stream — the **primary** — uses the project directory
itself as its worktree and tracks whatever branch is currently
checked out.

## 2. Look at the layout

The window is web-style, not IDE-style:

- **Stream tabs** (top row) — one tab per stream. The primary is
  pinned leftmost.
- **Thread tabs** (second row) — independent lines of work
  inside the active stream.
- **Rail HUD** (left) — your home base. Search trigger, active
  item, up-next, bookmarks, recent files, and a directory of
  every page you can open.
- **Page tabs** (center) — files, diffs, work items, notes,
  dashboards, the agent terminal, code-quality findings, and
  more. Each page has back/forward navigation and a Backlinks
  panel.
- **Status bar** (bottom) — branch chip, background-task
  indicator.

The amber accents are deliberate: anything amber is a primitive
oxplow added on top of git or Claude Code.

## 3. Send a prompt

Click the **Agent** tab in the active thread (the agent terminal
is always available per thread). Type a concrete prompt:

> List the files in this repo and write a one-paragraph summary
> of what the project does. Don't change any files.

The agent runs. The thread tab shows a yellow pulsing status dot
while it's working; it goes red when the agent is waiting on
you. Asking the agent a question that doesn't need file edits
just stops cleanly when it's answered — there's nothing to file.

Non-trivial Q&A is captured into a wiki note automatically (the
runtime nudges the agent to do this). Look for a new file under
`.oxplow/notes/` after the answer.

## 4. Send a prompt that changes files

Try something small and reversible:

> Add a single-line comment at the top of README.md that says
> "// touched by oxplow first-run". Don't change anything else.

A few things happen:

1. The agent files a work item before editing — it has to. The
   filing-enforcement hook denies edits unless an `in_progress`
   item is open.
2. The work item appears in the rail's **Active item** slot with
   a live status dot.
3. The agent edits `README.md` and snapshots it before/after.
4. The agent closes the item to `done` with a summary note.

Open **Local History** from the rail's Pages directory (or
right-click the file → Local History) and you'll see the
snapshot. Click to diff against current; click **Restore** to
revert just that file.

## 5. Add a second stream

Open the **+** in the stream tabs row. Give it a name and pick a
branch. Oxplow runs `git worktree add` next to the project root
and switches focus to it. The first stream keeps running
independently.

You now have two agents on two branches in the same repo, with
isolated working trees. Switch between them with the stream tabs.

## 6. Add a second thread

Click **+** in the thread tabs row to add a research / review
thread. By default new threads are read-only — their writes are
denied at the hook level. Use them to ask "how does X work" or
"trace this codepath" without risking file edits.

Promote a thread to writer from its tab kebab menu when you want
it to ship changes (only one writer per stream).

## 7. What to read next

- [Concepts](concepts.md) — streams, threads, pages, work items,
  notes, efforts.
- [Work queue](../guide/work-queue.md) — when to file work
  items and how the lifecycle behaves.
- [Notes](../guide/notes.md) — wiki notes, wikilinks, backlinks,
  how the agent uses them.
- [Agent control](../guide/agent-control.md) — Stop hook, write
  guard, filing enforcement.
