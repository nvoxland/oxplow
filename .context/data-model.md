# Data model

What this doc covers: the SQLite tables, their store classes, and the
single-`sort_index` queue invariant that ties work items, commit points,
and wait points together. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Storage

All persistence lives in one SQLite file under `.newde/state.sqlite`, opened
through `getStateDatabase()` (`src/persistence/state-db.ts`). Every store is
a thin class wrapping that connection. Schema changes go through versioned
migrations (`src/persistence/migrations.ts`) gated by `PRAGMA user_version`
— migrations are append-only; never edit a prior version.

## Tables and stores

### `streams` — `StreamStore` (`src/persistence/stream-store.ts`)

Top-level workspace context. One row per branch the user is working on.
Each stream owns:

- a worktree path (its own checkout under `.newde/worktrees/`)
- two tmux pane targets (`working` and `talking`)
- per-pane Claude resume session ids (so reconnecting picks up history)
- a `runtime_state.current_stream_id` pointer (singleton row, id=1)
- a `sort_index` column (migration v14) — streams are listed ordered by
  `sort_index, rowid`; drag-to-reorder in the StreamRail calls
  `reorderStreams(orderedStreamIds)` which reassigns sequential indexes.
  Emits a `stream.changed` event (kind: "reordered") so the UI can
  refresh.
- a `custom_prompt` column (migration v18, nullable TEXT) — per-stream
  standing instructions appended to the agent's system prompt after the
  global `agentPromptAppend` section. Set via `setStreamPrompt(streamId,
  prompt)` on `StreamStore`; IPC-exposed as `setStreamPrompt(streamId,
  prompt)`. Emits a `stream.changed` event (kind: "prompt-changed") so
  the UI re-fetches the full stream list.

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

### `batches` — `BatchStore` (`src/persistence/batch-store.ts`)

Units of work *within* a stream. Statuses: `active` (writer — may mutate the
worktree), `queued` (read-only, agents can run but writes are denied — see
[agent-model.md](./agent-model.md)'s write-guard section), `completed`
(archived). Exactly one batch per stream is `active`; the others are
`queued` or `completed`. A newly-seeded stream ships with one batch titled
`Default` (pre-v12 DBs called it `Current Batch`; migration v12 renames
the sort_index=0 row). The rolling `summary` field + `record_batch_summary`
MCP tool were removed in v13 — use the work-item log as the source of
truth instead.

`auto_commit` (migration v15, `INTEGER NOT NULL DEFAULT 0`) — when `true` on
the active batch, the stop-hook pipeline runs `git commit` directly whenever
settled work (`human_check`/`done` items) exists and no pending commit point
is already in the queue. No `commit_point` row is created; the UI is
notified via a `batch.changed`/`auto-committed` lifecycle event. Toggled via
`setAutoCommit(batchId, enabled)` on `BatchStore`; IPC-exposed as
`setAutoCommit(streamId, batchId, enabled)`. Surfaced in the Plan pane as an
"Auto-commit" toggle button next to the queue marker bar; while on, the
"+ Commit Point" button is hidden.

`custom_prompt` (migration v18, nullable TEXT) — per-batch standing
instructions appended to the agent's system prompt after the stream-level
`custom_prompt`. Set via `setBatchPrompt(batchId, prompt)` on `BatchStore`;
IPC-exposed as `setBatchPrompt(streamId, batchId, prompt)`. Emits a
`batch.changed` event (kind: "prompt-changed") so the UI refreshes batch
state.

### `work_items` — `WorkItemStore` (`src/persistence/work-item-store.ts`)

The actual TODO list. Kinds: `epic`, `task`, `subtask`, `bug`, `note`.
Statuses: `ready`, `in_progress`, `human_check`, `blocked`, `done`,
`canceled`, `archived`. `archived` is a terminal state that hides the item
from the default Work panel view — archived rows fold into the Done
section's bucketing but aren't rendered unless the user flips the "Show
archived (N)" toggle in the Done section header. The same header carries
an "Archive all" action that bulk-archives every visible Done/Canceled
row. `listReady`'s blocker check treats archived the same as done/
canceled. `parent_id` chains items under epics. `acceptance_criteria` is
plain text (one criterion per line). Work-item links express dependencies
(`blocks`, `discovered_from`, `relates_to`, …) via the `work_item_links`
join table.

`batch_id` is nullable — items with `batch_id IS NULL` belong to the
**backlog** (a global, stream-less queue). The store API uses the constant
`BACKLOG_SCOPE` as a sentinel string in event payloads so listeners can
distinguish backlog changes from in-batch changes.

`note_count` is a computed column added to every `WorkItem` returned by the
store (via COUNT subquery over `work_note`). It drives the note badge on
list rows and is always 0 when no notes exist.

### `work_note` — `WorkItemStore.getWorkNotes()` (`src/persistence/work-item-store.ts`)

Structured notes attached to individual work items. Each row has `id`,
`work_item_id`, `body`, `author` (free-form string, e.g. "agent", "user"),
and `created_at`. Created via migration v17. The store exposes
`getWorkNotes(itemId)` returning rows sorted by `created_at ASC`. The UI
calls `getWorkNotes` via the `getWorkNotes(itemId)` IPC method when the
edit modal opens; the modal renders a read-only "Notes" section. Agent and
user note writes go through `work_item_events` (event_type = 'note') for
now — `work_note` is the dedicated query-friendly table for structured note
display.

### `commit_point` — `CommitPointStore` (`src/persistence/commit-point-store.ts`)

Markers in the queue that say "commit at this point." Status:
`pending → done`. `mode` is `approve` (default) or `auto`. In approve
mode the agent inspects the diff, drafts a message in its chat reply,
asks the user to approve, and calls `newde__commit` once they do —
that runs `git commit` synchronously and flips the point to `done`. In
auto mode the runtime commits immediately with a generated message.
Drafted messages live only in chat; the row just stores mode, status,
and (once committed) the sha. See [agent-model.md](./agent-model.md)
for the Stop-hook flow.

### `wait_point` — `WaitPointStore` (`src/persistence/wait-point-store.ts`)

Markers that pause auto-progression. Status: `pending → triggered`.
Optional `note` shown to the user. Once triggered (the agent stopped at
this point), the marker is "consumed" — the next Stop hook treats it as
past, so prompting the agent at all resumes auto-progression. There is no
"continue" button.

### `agent_turn` — `TurnStore` (`src/persistence/turn-store.ts`)

One row per agent turn (UserPromptSubmit → Stop). Captures the prompt, the
sole-in-progress work item if any, and the Claude session id. The Stop
handler also sums assistant-message `usage` from the session's jsonl
transcript for the turn's time window and stores `input_tokens`,
`output_tokens`, and `cache_read_input_tokens`. Used by the Activity tab
and by file-change attribution.

### `batch_file_change` — `FileChangeStore` (`src/persistence/file-change-store.ts`)

Per-batch log of file mutations attributed to either a hook (Write/Edit/
MultiEdit/NotebookEdit PostToolUse) or fs-watch (anything else). Carries
turn_id and work_item_id when known. Drives the file-change indicators in
the project pane and the per-turn file filter. Each row gets a nullable
`snapshot_id` pointing at the `file_snapshot` that absorbed the change
(backfilled when a turn-start or turn-end snapshot flushes).

Read API: `listForBatch(batchId)`, `listForTurn(turnId)`, and
`listForWorkItem(itemId)` (used by the Work Item edit modal's
"Files changed" section + "Show in history" button — the latter opens
the Local History bottom panel and selects the snapshot matching the
most recent turn that touched files for the item).

### `file_snapshot` + `snapshot_entry` — `SnapshotStore` (`src/persistence/snapshot-store.ts`)

Metadata for content-addressed file snapshots. `file_snapshot` holds
one row per flush (kind, turn/batch fk, parent pointer, timestamp).
`snapshot_entry` holds the per-path rows for each snapshot: `path`,
`hash`, `mtime_ms`, `size`, `state`. Cascades from `file_snapshot`
delete entries automatically. Walking the parent chain reconstructs
the full file set.

Blobs live on disk at `.newde/snapshots/objects/xx/yyyy…` (sha256
addressed, shared across streams for dedup). Kinds: `turn-start` or
`turn-end`; the first turn-start on a stream doubles as its baseline.

Entry states:
- `present`: file captured, `hash` points at a real blob.
- `deleted`: tombstone — file was gone at flush time.
- `oversize`: file existed but exceeded `snapshotMaxFileBytes`; no
  blob, but `mtime_ms` and `size` are tracked so diffs still report
  "it changed (by this much)" even without content.

The `streams.current_snapshot_id` column holds the stream's latest
snapshot — it's the parent for the next flush.

Read API (surfaced via IPC — see `ipc-and-stores.md`):

- `listSnapshotsForStream(streamId, limit)` — newest-first.
- `getSnapshotSummary(id)` → `{ snapshot, files: {path: {entry,
  kind}}, counts }`; classifies each manifest entry as
  created/updated/deleted relative to the parent chain.
- `getSnapshotFileDiff(id, path)` — resolves "before" from the
  parent chain and "after" from this snapshot.
- `getSnapshotPairDiff(beforeId, afterId, path)` — arbitrary pair.
- `getTurnSnapshots(turnId)` — `{ start, end }` for a turn, used by
  `runtime.getTurnFileDiff`.

**Retention.** `SnapshotStore.cleanupOldSnapshots(retentionDays)`
deletes snapshots older than the cutoff (default 7 days, configurable
via `newde.yaml`'s `snapshotRetentionDays`; `0` disables pruning),
except:

- the most recent snapshot per stream is always kept;
- anything `streams.current_snapshot_id` points at is always kept.

After snapshot deletion, `gcBlobs()` sweeps `.newde/snapshots/objects/`
and removes any blob whose sha isn't referenced by a surviving
manifest. Descendants whose parent was deleted keep pointing at a
missing id — `resolvePath` simply stops walking there, so the oldest
surviving snapshot effectively becomes a new baseline for files it
touches. The trade-off: ancient history drops; recent diffs stay
intact. The blob store is shared across all streams in the project
(`.newde/snapshots/objects/`), so GC runs at the project level and
naturally dedupes identical content across branches.

Cleanup runs at runtime startup and again once every 24 hours via
`runtime.runSnapshotCleanup` (wired in `initialize()`, cleared in
`dispose()`).

**Ignoring generated directories.** The fs-watcher and the snapshot
seeder share one filter: `shouldIgnoreWorkspaceWatchPath` in
`src/git/workspace-watch.ts`. It covers `.git/`, `.newde/logs/`,
`.newde/worktrees/`, and a hardcoded list of common build/cache dir
names (`node_modules`, `dist`, `build`, `target`, `.next`, `.turbo`,
`.cache`, `.venv`, `__pycache__`, …). Users can extend the list via
`generatedDirs: [...]` in `newde.yaml` — names are single path
segments matched anywhere in the relative path, and apply to both
the workspace watcher and the snapshot store. No changes to
existing snapshots on toggle; newly ignored paths simply stop
appearing in future dirty sets.

## The shared `sort_index` queue

The most important invariant in the data model: **`work_items.sort_index`,
`commit_point.sort_index`, and `wait_point.sort_index` all live in the
same numeric space, scoped per batch.**

- `runtime.reorderBatchQueue(streamId, batchId, entries)` rewrites all
  three tables' `sort_index` values in one operation. Each entry is
  `{ kind: "work" | "commit" | "wait", id }` and gets `sort_index = position`.
- The UI merges work items + commit points + wait points by `sort_index`
  to render a single ordered list (`WorkGroupList` in `PlanPane.tsx`).
- `runtime.findActiveCommitPoint` and `findActiveWaitPoint` walk the
  merged list to find the lowest-`sort_index` non-terminal marker whose
  preceding work items are all `done`/`canceled`. That's the marker the
  Stop-hook pipeline acts on.

There is **no foreign key** linking commit/wait points to specific work
items. Reordering the queue immediately changes which work items a marker
"covers" (everything before it, not yet covered by an earlier marker),
with no migration step.

**Visual vs persistence order for Human Check.** Every section in
`WorkGroupList` renders ascending by `sort_index` *except* Human Check,
which renders descending (newest-finished items surface on top). The
underlying `sort_index` space is still a single ascending line — the
section is only flipped at render time. When a drag-reorder persists a
new order, `finalizeReorderIds` in `plan-utils.ts` reverses the
humanCheck subsequence so the `reorderItems` / `reorderBatchQueue`
"sort_index = position" rule produces the intended visual result. Any
new section with a non-ascending display must either do the same
reversal dance or get its own flat list.

Constraint: commit and wait points cannot be the very first queue entry —
they have nothing to fire after. Enforced both in the runtime
(`createCommitPoint` / `createWaitPoint` throw if the batch has zero work
items) and in the UI (buttons are disabled with an explanatory tooltip).

## Status diagrams (text)

```
work item:    ready ─► in_progress ─► human_check ─► done ─► archived
                   ╰─────────► blocked
                   ╰─────────► canceled ─► archived

commit point: pending ─► done

wait point:   pending ─► triggered                     (consumed)
```

## Change events

Every store has a `subscribe(listener)` for in-process listeners,
implemented via the shared `StoreEmitter` helper
(`src/persistence/store-emitter.ts`). The emitter snapshots its
listener set before iterating so a listener that unsubscribes itself
during emission doesn't skip subsequent subscribers, and a throwing
listener is logged-and-skipped rather than killing the whole emit.

The runtime relays each store's changes onto the typed EventBus
(`src/core/event-bus.ts`) as `*.changed` events:

- `workspace.changed`, `git-refs.changed`, `workspace-context.changed`
- `work-item.changed`, `backlog.changed`, `batch.changed`, `turn.changed`
- `file-change.recorded`, `file-snapshot.created`, `agent-status.changed`
- `commit-point.changed`, `wait-point.changed`
- `hook.recorded`, `config.changed`

UI components subscribe via `subscribeNewdeEvents()` (or scoped helpers
like `subscribeWorkspaceEvents`, `subscribeGitRefsEvents`) in
`src/ui/api.ts`. See [ipc-and-stores.md](./ipc-and-stores.md) for how to
plumb a new event end-to-end.

## UI-only state worth naming

A few UI surfaces hold non-persisted state that callers (probes,
docs, future stores) reference by name. Listed here so renames touch
the docs in one diff:

- **Files-pane filter mode** — `FilterMode = "all" | "uncommitted" |
  "branch" | "unpushed" | "turn"` lives in `ProjectPanel` state and
  drives the file-tree visibility filter. The eye-icon trigger button
  is `data-testid="files-filter-toggle"`; each popover option is
  `data-testid="files-filter-option-<value>"` (e.g.
  `files-filter-option-uncommitted`). `branch` / `unpushed` auto-
  fall back to `uncommitted` if the underlying scope disappears (no
  branch base, no upstream).

## Related

- [ipc-and-stores.md](./ipc-and-stores.md) — adding new stores and IPC.
- [agent-model.md](./agent-model.md) — how the agent acts on this data.
- [git-integration.md](./git-integration.md) — `gitCommitAll` reads
  approved commit points and writes back the resulting sha.
