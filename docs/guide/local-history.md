# Local History

Every time the agent runs, oxplow snapshots the files it
touches — before and after. The collected snapshots are **Local
History**. Think of it as a per-file, per-effort timeline,
independent of git, that lets you see exactly what changed and
roll any individual file back without disturbing the rest.

## Where snapshots come from

A snapshot is captured when:

- A file is about to be written by the agent (the "before"
  snapshot).
- The turn ends and the file is in a different state than its
  before-snapshot (the "after" snapshot).

Snapshots are stored under `.oxplow/` and attributed to the
**effort** that produced them.

## Effort

An effort is one attempt at one work item. If you reopen a
`human_check` item to redo it, that opens a *second* effort on
the same item. The Local History modal groups snapshots by
effort, so you can see the full history of attempts against a
single concern.

## Opening Local History

- Right-click a file in the file browser → **Local History**.
- Or right-click a work item in the Plan pane → **Local
  History** to see every file that effort touched.

The modal shows a chronological list. Each row is a snapshot:
file path, timestamp, effort it belongs to, and the work item
title.

## Comparing and restoring

Click a snapshot to open the diff against the current state.

Click **Restore** to overwrite the current file with the
snapshot's contents. Restoring is targeted — only that file is
affected. The rest of your working tree is untouched.

This is the main "undo" path for agent work. It is *not* the
same as `git revert` or `git reset`:

- `git reset` rewinds your *whole* working tree (and history,
  if you're not careful).
- Restoring from Local History rewinds *one file* to a known
  state, with no impact on git history.

Use git for committed history. Use Local History for the
working-tree shape between commits.

## Attribution and parallel efforts

When two work items run in parallel (rare but possible), oxplow
needs a way to know which item touched which file so the modal
shows the right snapshots in the right place. That's why the MCP
tools take a `touchedFiles` argument when an item closes:
attribution information for Local History.

You don't have to think about this — the agent handles it. It's
mentioned here so the file lists in the modal make sense when
you see them.

## Cleanup

Snapshots accumulate. Old efforts under closed items are pruned
on a schedule (configurable; defaults are sensible). If you want
to keep a snapshot indefinitely, copy the file out — pruning
won't ask before it removes data.
