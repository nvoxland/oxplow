# Local History

Local History records working-tree state between commits. It helps you
understand an agent's changes and compare or restore individual files.
Capture does not require an open task.

## Where snapshots come from

Oxplow observes file changes within each stream's worktree. Agent turn
boundaries request captures; startup and background capture also pick up
changes made outside a turn. A snapshot groups the changed files, and
available content is stored under `.oxplow/snapshots/`.

Each stream has its own history. A snapshot is evidence of observed
state, rather than a declaration by the agent of what it edited.

## What pins to git

Snapshots record their relationship to version-control history. A
snapshot taken at a clean revision can be tied exactly to that revision;
uncommitted state must be distinguished from an exact commit match.
This lets you move between the work observed during development and the
committed result.

## Turns and requests

A **turn** is one run of an agent, from a prompt to its stop. A
**request** is the turn that started something, plus every turn that
followed from it, such as a background job's report or a subagent's
hand-back. Work recorded against a turn, such as tool calls, tokens and
test runs, belongs to the request. A request can link to tasks and to
commits. A finished request does not automatically mark a task done.

Each turn's files come from joining the turn with its stream's snapshots.
Snapshots are the worktree's own record, so shell commands and
formatters contribute changes as well as edit tools. The captured states
around a turn provide the bracket for reviewing it.

## The Local History dashboard

Open **Local History** from the launcher (++cmd+p++) to browse snapshots
and follow their associated changes and requests. File pages also offer
history for that path. The available analysis sections depend on the
extensions enabled in the project; file history and diffs provide the
underlying evidence.

## File-page integration

A file page offers history for its path. Select a captured version to
inspect the earlier state or open its diff against the current file.

## Comparing and restoring

Open a captured version to compare it with the current file. **Restore**
writes that version into the working tree. It affects the selected file
and leaves git history intact.

Use git for committed history and Local History for state between
commits. Check the resulting diff after a restore, especially if newer
work has changed the same file.

## Filtering out generated paths

The `generated` configuration in `.oxplow/project.yaml` has `exclude`
and `include` patterns. It identifies build output that should not
be treated as authored development work. See
[Settings](../reference/settings.md#generated-paths) for the schema and
its relationship to `.gitignore`.

## Cleanup

Snapshot content has a retention window. Background cleanup expires
old content and removes orphaned blobs; the historical record of a
capture is distinct from the retained file contents. Settings also
select whether the snapshot provider keeps every version or tracks
changes only.

See [Settings](../reference/settings.md#snapshot-retention). Copy any
version you need to keep outside that retention policy.
