# Local History

Every time the agent works on a task, oxplow snapshots the
files it touched — before and after. The collected snapshots
are **Local History**: a per-stream timeline of working-tree
state, independent of git, that lets you see what changed,
inspect a diff against any earlier snapshot, and roll any
individual file back without disturbing the rest.

## Where snapshots come from

`SnapshotCaptureService` is a request-driven dirty-set manager.
It listens to fs-watch events on the stream's worktree, marks
paths dirty as they change, and flushes the dirty set into a
batched capture when:

- An effort transitions in_progress → done / blocked, or
- A startup sweep runs (catching changes made while oxplow
  wasn't running), or
- An explicit `request_snapshot()` fires for any other reason.

A capture is a single parent `snapshot` row that groups one
`file_snapshot` row per dirty path. Blobs are content-addressed
(xxh3-128) and stored under `.oxplow/snapshots/`. The startup
sweep short-circuits per file when `(size, mtime)` match the
prior capture, so cold start on a large repo doesn't pay the
hash cost on every file.

Captures are per-stream — each stream has its own worktree, so
its snapshots don't bleed into another stream's history.

## What pins to git

When the worktree is clean at capture time (no tracked-file
changes, no untracked-non-ignored files), the snapshot is
pinned to the current `git_commit` SHA. HEAD moves re-stamp
the latest snapshot so the pin stays current.

That pin is what makes the "By git commit" view in the
dashboard possible — clean-worktree captures group under the
commit they were taken against; dirty-worktree captures get
their own "Uncommitted" group.

## Effort

An effort is one open-and-close cycle of a task: opens when
the task transitions to `in_progress`, closes when it
transitions to `done` / `blocked` (with a snapshot id pinned at
each end). The Local History dashboard surfaces which efforts
were in-flight or just completed in each snapshot's window.

When the agent closes an effort it passes its declared
`touched_files`. The runtime cross-checks that list against
the snapshot bracket diff and asks the agent to `amend_effort`
if the two disagree — see
[Agent control](agent-control.md#stop-hook).

## The Local History dashboard

Open from the launcher (++cmd+p++) → **Local History**.
The page replaces the old bottom-rail SnapshotsPanel; it's now
a full page tab with two view modes:

- **Recent.** Snapshots in reverse chronological order,
  grouped by a sliding activity window. Each row carries the
  pinned commit (or "uncommitted"), file counts, in-flight or
  just-completed effort labels, and wiki-edit badges where
  applicable.
- **By git commit.** Snapshots grouped under the commit each
  pins to, with branch and tag chips in the group header.
  Multiple snapshots can share a commit (e.g. wiki edits with
  no other tracked changes); a dedicated **Uncommitted** group
  collects dirty-worktree captures.

Clicking a row takes you into its diff.

## Per-snapshot detail page

Clicking a snapshot row opens **Snapshot detail**, which has
full ChangeAnalysisPanel parity with the Git commit page:

- The *Look here first* card ranks changed files by
  interestingness (churn × complexity × tests-missing ×
  duplication).
- The change treemap groups cells by architectural zone
  (backend / frontend / config / docs / tests / build).
- Duplication and per-function metrics cards work against the
  snapshot's bracket.
- Status filter (added / modified / deleted) and per-file
  navigation reuse the same primitives as commit pages.

The page also surfaces:

- **Efforts in progress at this snapshot** — every effort
  whose start/end snapshot bracket includes this id, with
  links to each task.
- **Prev / next snapshot** navigation in the chrome.

## File-page integration

Open any tracked file. The chrome carries a per-snapshot
history dropdown listing every capture of that path, in
descending order, with the pinned commit / effort label per
row. Click an entry to jump to the snapshot detail page
scoped to that file, or open a diff against the current
working-tree state.

Right-click a file → **Local History** does the same thing
from the file tree.

## Comparing and restoring

The detail page's file rows open a Monaco diff editor against
the working-tree state. From that diff, **Restore** overwrites
the working tree with the snapshot's contents.

Restore is targeted — only that file is affected. The rest of
the working tree is untouched.

This is the main "undo" path for agent work. It is *not* the
same as `git revert` or `git reset`:

- `git reset` rewinds your *whole* working tree (and history,
  if you're not careful).
- Restoring from Local History rewinds *one file* to a known
  state, with no impact on git history.

Use git for committed history. Use Local History for the
working-tree shape between commits.

## Filtering out generated paths

Snapshots respect the `generated` list in `.oxplow/project.yaml`:

- The capture pipeline doesn't watch them in the first place.
- Read paths (snapshot list IPCs, effort file-reviews, the
  Local History dashboard, file detail dropdowns) also filter
  the live config, so paths added to `generated` after they
  were captured drop out of the UI immediately.

See [Settings](../reference/settings.md#generated-paths) for
the schema.

## Cleanup

Snapshots accumulate. Old snapshot rows are pruned on a 24-hour
schedule and orphaned blobs in `.oxplow/snapshots/` get GC'd at
the same time. Retention is configurable from the project's
settings page; defaults are sensible. The background task HUD
surfaces sweep and cleanup runs so you can see when they fired.

If you want to keep a snapshot indefinitely, copy the file
out — pruning won't ask before it removes data.
