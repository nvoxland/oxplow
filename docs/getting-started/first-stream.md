# Your first stream

This page walks through the first five minutes: open a project,
start a stream, send a prompt, watch what oxplow does, accept the
work.

## 1. Open the project

**File → Open Project** and pick a git repo. Oxplow scopes itself
to that directory — it will not climb upward looking for an
enclosing repo. Whatever you opened *is* the workspace.

The first time you open a project, oxplow creates `.oxplow/` next
to your `.git/`:

```
.oxplow/
  state.sqlite           # work items, threads, snapshots, settings
  worktrees/             # extra checkouts for non-primary streams
```

The first stream — the **primary** stream — uses the project
directory itself as its working tree. It tracks whatever branch is
currently checked out.

## 2. Look at the layout

The window has four zones:

- **Stream rail** (left) — one tab per stream. The primary is
  pinned leftmost.
- **Plan pane** (left of center) — work queue, commit/wait points,
  notes, agent thread list.
- **Editor + file browser** (center) — Monaco-based editor with a
  custom React file tree.
- **Terminal pane** (bottom) — pty bridge for the agent and any
  shells you open.

The amber accents are deliberate: anything amber is a primitive
oxplow added on top of git or Claude Code.

## 3. Send a prompt

Click into the agent terminal at the bottom of the primary stream.
Type something concrete. For a tour:

> List the files in this repo and write a one-paragraph summary of
> what the project does. Don't change any files yet.

Hit enter. The agent runs. Oxplow's Stop hook captures the turn
and shows it as a live row in the Plan pane while it's in flight.

When the agent is done, the live row disappears — turns themselves
aren't durable rows, only work items are.

## 4. Send a prompt that actually changes files

Try something small and reversible:

> Add a single-line comment at the top of README.md that says
> "// touched by oxplow first-run". Don't change anything else.

The agent will edit the file. Oxplow snapshots `README.md` before
and after, attached to whatever work item is in flight (or to a
synthetic effort if there isn't one).

Open the **Local History** modal (right-click the file → Local
History) and you'll see the snapshot. Click it to see the diff.
Click **Restore** to revert if you don't like it.

## 5. Add a second stream

Click **+** in the stream rail. Give it a name and a branch. Oxplow
creates a new worktree under `.oxplow/worktrees/` and switches
focus to it. The first stream keeps running independently.

You now have two agents on two branches in the same repo, with
isolated working trees. Switch between them with the rail tabs.

## 6. What to read next

- [Concepts](concepts.md) — streams, threads, the writer rule,
  worktrees.
- [Work queue](../guide/work-queue.md) — when to file work items
  and how the lifecycle behaves.
- [Commit & wait points](../guide/commit-and-wait-points.md) —
  inline gates that let you steer without typing.
