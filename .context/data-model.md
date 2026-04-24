# Data model

What this doc covers: the SQLite tables, their store classes, and the
single-`sort_index` queue invariant that ties work items, commit points,
and wait points together. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Storage

All persistence lives in one SQLite file under `.oxplow/state.sqlite`, opened
through `getStateDatabase()` (`src/persistence/state-db.ts`). Every store is
a thin class wrapping that connection. Schema changes go through versioned
migrations (`src/persistence/migrations.ts`) gated by `PRAGMA user_version`
— migrations are append-only; never edit a prior version.

## Tables and stores

### `streams` — `StreamStore` (`src/persistence/stream-store.ts`)

Top-level workspace context. Exactly one row per user-facing stream tab.
Each stream owns:

- a `kind` column (migration v34) — `"primary" | "worktree"`:
  - **primary**: the repo itself. `worktree_path === projectDir`,
    `title === projectBase` (never rewritten), created exactly once at
    startup by `ElectronRuntime.initialize()` via `StreamStore.findPrimary()`.
    Cannot be deleted (`StreamStore.deleteStream()` throws for `kind === "primary"`).
  - **worktree**: a real git worktree under `.oxplow/worktrees/<branch>/`
    created by `createStream()` via `ensureWorktree()`. Title defaults
    to the branch name; the runtime rewrites the title when the branch
    switches only if the old title matched the old branch (preserves
    user renames).
- a `branch` / `branch_ref` pair that is **not** pinned — any stream can
  switch branches. Updated by `StreamStore.setStreamBranch(streamId,
  branch, branchRef)`, which emits `stream.changed` (kind:
  `"branch-changed"`). The runtime drives it from two sites:
  `ElectronRuntime.checkoutStreamBranch(streamId, branch)` (user-triggered
  via "Switch branch…" in the StreamRail context menu) and
  `maybeSyncStreamBranch(streamId)` (fired by every `git-refs.changed`
  event so external `git checkout` in a worktree is picked up live).
  Git-level errors (dirty tree, missing branch, branch already checked
  out in another worktree) bubble through unchanged so the UI inline
  error shows git's own message.
- a worktree path (projectDir for primary; `.oxplow/worktrees/<slug>/`
  for worktree kind — the `<slug>` is fixed at creation and does not
  rename on branch switch)
- two tmux pane targets (`working` and `talking`)
- per-pane Claude resume session ids (so reconnecting picks up history)
- a `runtime_state.current_stream_id` pointer (singleton row, id=1)
- a `sort_index` column (migration v14) — streams are listed ordered by
  `sort_index, rowid`; drag-to-reorder in the StreamRail calls
  `reorderStreams(orderedStreamIds)` which reassigns sequential indexes.
  The UI enforces **primary-first** regardless of sort_index (the
  primary tab can't be dragged and nothing can drop before it). Emits a
  `stream.changed` event (kind: "reordered") so the UI can refresh.
- a `custom_prompt` column (migration v18, nullable TEXT) — per-stream
  standing instructions appended to the agent's system prompt after the
  global `agentPromptAppend` section. Set via `setStreamPrompt(streamId,
  prompt)` on `StreamStore`; IPC-exposed as `setStreamPrompt(streamId,
  prompt)`. Emits a `stream.changed` event (kind: "prompt-changed") so
  the UI re-fetches the full stream list.

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

### `threads` — `BatchStore` (`src/persistence/thread-store.ts`)

Units of work *within* a stream. Statuses: `active` (writer — may mutate the
worktree), `queued` (read-only, agents can run but writes are denied — see
[agent-model.md](./agent-model.md)'s write-guard section), `completed`
(archived). Exactly one thread per stream is `active`; the others are
`queued` or `completed`. A newly-seeded stream ships with one thread titled
`Default` (pre-v12 DBs called it `Current Thread`; migration v12 renames
the sort_index=0 row). The rolling `summary` field + `record_batch_summary`
MCP tool were removed in v13 — use the work-item log as the source of
truth instead.

`auto_commit` (migration v15, `INTEGER NOT NULL DEFAULT 0`) — when `true` on
the active thread, the stop-hook pipeline runs `git commit` directly whenever
settled work (`human_check`/`done` items) exists and no pending commit point
is already in the queue. No `commit_point` row is created; the UI is
notified via a `thread.changed`/`auto-committed` lifecycle event. Toggled via
`setAutoCommit(threadId, enabled)` on `BatchStore`; IPC-exposed as
`setAutoCommit(streamId, threadId, enabled)`. Surfaced in the Plan pane as an
"Auto-commit" toggle button next to the queue marker bar; while on, the
"+ Commit Point" button is hidden.

`custom_prompt` (migration v18, nullable TEXT) — per-thread standing
instructions appended to the agent's system prompt after the stream-level
`custom_prompt`. Set via `setBatchPrompt(threadId, prompt)` on `BatchStore`;
IPC-exposed as `setBatchPrompt(streamId, threadId, prompt)`. Emits a
`thread.changed` event (kind: "prompt-changed") so the UI refreshes thread
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

`thread_id` is nullable — items with `thread_id IS NULL` belong to the
**backlog** (a global, stream-less queue). The store API uses the constant
`BACKLOG_SCOPE` as a sentinel string in event payloads so listeners can
distinguish backlog changes from in-thread changes.

`author` (migration v26, nullable TEXT) — semantic origin of the row,
distinct from `created_by` (which just classifies the SQL writer as
`user`/`agent`/`system`). Values: `'user'` (explicit user-initiated
create), `'agent'` (explicit agent `create_work_item` /
`file_epic_with_children` call), or `NULL` (legacy rows). Pre-v29 DBs
also held `'agent-auto'` rows synthesized by the removed auto-file
listener; migration v29 cancels any such still-in_progress rows, and
the read path maps the legacy string to `null` so older terminal rows
continue to load under the narrowed enum.

`note_count` is a computed column added to every `WorkItem` returned by the
store (via COUNT subquery over `work_note`). It drives the note badge on
list rows and is always 0 when no notes exist.

### `work_note` — `WorkItemStore.getWorkNotes()` / `listThreadNotes()` (`src/persistence/work-item-store.ts`)

Structured notes, either item-scoped or thread-scoped. Each row has `id`,
nullable `work_item_id`, nullable `thread_id`, `body`, `author` (free-form
string, e.g. "agent", "user", "explore-subagent"), and `created_at`. A
CHECK enforces that **exactly one** of `work_item_id` / `thread_id` is
non-NULL. Created via migration v17 (item-scoped rows) and broadened in
migration v25 to allow thread-scoped rows (nullable `work_item_id`, new
`thread_id` column).

- **Item-scoped rows** (`work_item_id` set, `thread_id` NULL) back
  `getWorkNotes(itemId)` returning rows sorted by `created_at ASC`. The UI
  calls this via the `getWorkNotes(itemId)` IPC method when the edit modal
  opens; the modal renders a read-only "Notes" section. Agent and user
  note writes today still also fan out through `work_item_events`
  (event_type = 'note') — `work_note` is the dedicated query-friendly
  table for structured note display.

- **Thread-scoped rows** (`thread_id` set, `work_item_id` NULL) are the
  landing spot for `oxplow__delegate_query` Explore-subagent findings. The
  delegate tool pre-allocates a row with empty body (via
  `addThreadNote`), passes the id into the subagent prompt, and the
  subagent fills the body by calling `oxplow__record_query_finding` (store
  method `updateThreadNoteBody`). The orchestrator reads them back via
  `oxplow__get_thread_notes` / `listThreadNotes(threadId, limit)` —
  reverse-chronological, capped at 100.

### `commit_point` — `CommitPointStore` (`src/persistence/commit-point-store.ts`)

Markers in the queue that say "commit at this point." Status:
`pending → done`. `mode` is `approve` (default) or `auto`. Both modes
now route through the agent: the Stop-hook directive asks the agent to
inspect the diff and draft a message. The only difference is the user-
approval gate — approve mode asks, auto mode commits in the same turn.
Either way the agent calls `oxplow__commit({ commit_point_id, message })`,
`git commit` runs synchronously, and the row flips to `done`. Drafted
messages live only in chat; the row stores mode, status, and (once
committed) the sha. See [agent-model.md](./agent-model.md) for the
Stop-hook flow.

`completed_at` (set when the row flips to `done`) is the cutoff for two
downstream queries:

- `CommitPointStore.getLatestDoneForThread(threadId)` — used by the
  `mcp__oxplow__tasks_since_last_commit` MCP tool to bound "what's
  changed since last commit?" for auto-commit drafting.
- The `buildAutoCommitMessage` fallback filters settled work items by
  `updated_at > completed_at` so its body reflects this commit's
  changes, not every settled item in the thread's history.

### `wait_point` — `WaitPointStore` (`src/persistence/wait-point-store.ts`)

Markers that pause auto-progression. Status: `pending → triggered`.
Optional `note` shown to the user. Once triggered (the agent stopped at
this point), the marker is "consumed" — the next Stop hook treats it as
past, so prompting the agent at all resumes auto-progression. There is no
"continue" button.

### `agent_turn` — `TurnStore` (`src/persistence/turn-store.ts`)

One row per agent turn (UserPromptSubmit → Stop). Captures the prompt,
answer, Claude session id, token usage, and the `start_snapshot_id` +
`end_snapshot_id` taken at the turn boundaries. The Activity tab builds
its per-turn file-change list by diffing those two snapshots. A turn has
no direct work-item FK anymore — efforts connect work items to turns via
the `work_item_effort_turn` join table (see below).

`task_list_json` (migration v30, nullable TEXT) stores the serialized
TaskCreate/TaskUpdate breakdown for the turn. Written incrementally by
the PostToolUse TaskCreate bridge on every call (not only at Stop) so
the Work panel's in_progress bucket can render the live sub-list on
the open-turn row. Persists on the row after the turn closes for a
later History view. `TurnStore.setTaskList(turnId, json)` emits a
`turn.changed` / `task-list-updated` event so UI subscribers refresh.
`TurnStore.listOpenTurns(threadId, sinceIso)` returns rows with
`ended_at IS NULL` AND `started_at >= sinceIso` — the runtime passes
its own `startedAt` as the cutoff to filter orphaned open turns from
prior runs out of the live list.

`produced_activity` (migration v31, nullable INTEGER) records whether
any mutation / filing / dispatch tool call fired during the turn — the
same in-memory `turnActivityByThread` flag that drives the Stop-hook's
ready-work suppression rule. Persisted by the runtime's Stop hook after
`closeTurn` (before `computeStopDirective` consumes the in-memory flag)
via `TurnStore.setProducedActivity(turnId, boolean|null)`: `1` =
activity, `0` = pure Q&A, NULL = unknown (pre-v31 rows, or turns where
UserPromptSubmit never fired). Drives the Work panel's "Recent answers"
section: `TurnStore.listRecentInactiveTurns(threadId, limit=10)`
returns closed turns with `produced_activity = 0`, newest-ended first.
NULL rows are excluded so legacy turns don't back-fill the list.

### `work_item_effort` + `work_item_effort_turn` — `WorkItemEffortStore` (`src/persistence/work-item-effort-store.ts`)

An **effort** is one `in_progress → human_check` (or done/canceled) cycle
of a work item. Columns: `work_item_id`, `started_at`, `ended_at`,
`start_snapshot_id`, `end_snapshot_id`. Auto-managed by the runtime on
`work-item.changed` status transitions:

- `→ in_progress` opens a new effort; a `task-start` snapshot is flushed
  and linked to `start_snapshot_id`.
- `in_progress → {human_check, done, canceled}` closes the effort; a
  `task-end` snapshot is flushed and linked to `end_snapshot_id`.

Re-opening a task (human_check → ready → in_progress) produces a second
effort. At most one open effort per work item at a time.

`work_item_effort_turn` is a many-to-many join so a single effort can
span multiple turns.

`work_item_effort_file` (v22) records per-effort write paths so parallel
subagents in one thread get distinct file lists instead of the union via
the snapshot pair-diff. Columns: `effort_id`, `path`, `first_seen_at`,
primary key `(effort_id, path)`. Rows come from the `touchedFiles`
payload on the `update_work_item` transition to `human_check`, not
from the PostToolUse hook (the previous heuristic couldn't attribute
writes when ≥2 efforts were in_progress). See agent-model.md's
"Per-effort write log" for the flow. Consumed by
`computeEffortFiles(effortStore, snapshotStore, effortId)` (exported
from `runtime.ts`): when ≥2 efforts share an end snapshot AND this
effort has ≥1 row here, the pair-diff is filtered to those paths;
0 rows → fall back to raw pair-diff ("assume all"); 1 effort → raw
pair-diff. The runtime writes a link row (a) for every
currently-open effort when a turn opens and (b) for the currently-open
turn when an effort opens mid-turn.

Read API: `listEffortsForWorkItem(itemId)`, `listOpenEfforts()`,
`listEffortsForSnapshot(snapshotId)`, `listTurnsForEffort(effortId)`,
`listEffortsForTurn(turnId)`, `listEffortsForPath(path)` (closed
efforts that touched `path` via `work_item_effort_file`, joined to the
owning work item's title/status, newest-first by `ended_at` — drives
the local-blame overlay described in `.context/editor-and-monaco.md`).
`createWorkItemApi` exposes `listWorkItemEfforts(itemId)` which returns
per-effort rows with pre-joined start/end snapshot metadata and the
list of changed paths (computed from the pair diff).

**Commit↔item attribution is intentionally NOT tracked.** A
`work_item_commit` junction existed briefly (migration v27) but was
removed in v28. Users commit outside oxplow all the time (IDE buttons,
CLI, CI rebases, merges, squashes) and oxplow has no authoritative hook
there; the best the runtime could do was a heuristic ("items whose
efforts closed in the window since the last done commit_point"), which
silently misattributes when work is split across multiple commits,
committed earlier, or touched by reopened efforts. A blame-style
feature built on that data would lie more often than it'd be useful.
The supplementary `mcp__oxplow__tasks_since_last_commit` tool stays —
it's advisory context for the agent drafting a commit message, not a
persisted link — and `commit_point.commit_sha` stays (1:1 with the
point the runtime itself closed). If a future feature wants "show me
commits for this item," the answer is to scope `git log` by the files
in `work_item_effort_file`.

### `file_snapshot` + `snapshot_entry` — `SnapshotStore` (`src/persistence/snapshot-store.ts`)

Time-ordered, self-contained snapshots. `file_snapshot` columns:
`id, stream_id, worktree_path, version_hash, source, created_at`.
`snapshot_entry` holds the per-path rows: `path, hash, mtime_ms, size,
state`.

`source` is one of `task-start | task-end | turn-start | turn-end |
startup | external`. `version_hash` is a SHA-256 over the canonical
`(path, hash, size, state)` entry set — `mtime_ms` is deliberately
excluded so touching a file without changing its bytes doesn't produce
a new snapshot. Deleted files have no `snapshot_entry` row at all (the
"entry missing" case is the deletion signal); readers collapse
"absent" and the old `state="deleted"` cases into one branch.

**Dedup on flush.** `flushSnapshot()` computes the next snapshot's
`version_hash` (reusing the most recent snapshot's entries for any path
not listed in `dirtyPaths`), and if that hash matches the newest
existing snapshot for the stream, it returns `{ created: false, id:
<existing> }` instead of inserting a new row. `dirtyPaths` is an
optimizer hint — when null the entire worktree is walked.

**No parent chain.** Snapshots have no `parent_snapshot_id`; the
"previous" snapshot for diff purposes is just the most recent
`file_snapshot` row with `created_at < target.created_at` for the same
stream. `getSnapshotSummary(id, previousId?)` returns created/updated/
deleted counts relative to that previous snapshot (or an explicit one
if provided); `getSnapshotPairDiff(beforeId, afterId, path)` serves
arbitrary pair diffs.

**Baseline is hidden from Local History.** The first snapshot per
stream has no predecessor, so there's nothing to diff against and
nothing meaningful to show. `listSnapshotsForStream` excludes it
(subsequent snapshots use it as their "previous" via
`getSnapshotSummary`). The baseline still lives in the DB — only the
UI list skips it.

**Rows come with pre-joined labels.** `listSnapshotsForStream` joins
against `work_item_effort` and `agent_turn` in a single query to
populate `label` + `label_kind` on each `FileSnapshot`. Effort links
win over turn links (task title + " — start"/" — end"); effort-end
wins over effort-start when the same snapshot is both. Unlinked
snapshots get `label: null` and the UI falls back to the `source`
column.

Blobs live on disk at `.oxplow/snapshots/objects/xx/yyyy…` (sha256
addressed, shared across streams for dedup).

Entry states:
- `present`: file captured, `hash` points at a real blob.
- `oversize`: file existed but exceeded `snapshotMaxFileBytes`; no blob,
  but `mtime_ms` and `size` are tracked.

A deleted file has no `snapshot_entry` row — readers treat a missing
entry as the deletion signal. Migration v24 drops any legacy
`state='deleted'` tombstones.

**Retention.** `SnapshotStore.cleanupOldSnapshots(retentionDays)`
deletes snapshots older than the cutoff (default 7 days, configurable
via `oxplow.yaml`'s `snapshotRetentionDays`; `0` disables pruning). The
most recent snapshot per stream is always kept. `gcBlobs()` then sweeps
`.oxplow/snapshots/objects/` and removes any blob whose sha isn't
referenced by a surviving manifest. The blob store is shared across all
streams (`.oxplow/snapshots/objects/`), so GC runs at the project level
and dedupes identical content across branches.

Cleanup runs at runtime startup and again once every 24 hours via
`runtime.runSnapshotCleanup` (wired in `initialize()`, cleared in
`dispose()`).

**Ignoring generated directories.** The fs-watcher and the snapshot
seeder share one filter: `shouldIgnoreWorkspaceWatchPath` in
`src/git/workspace-watch.ts`. It covers `.git/`, `.oxplow/logs/`,
`.oxplow/worktrees/`, and a hardcoded list of common build/cache dir
names (`node_modules`, `dist`, `build`, `target`, `.next`, `.turbo`,
`.cache`, `.venv`, `__pycache__`, …). Users can extend the list via
`generatedDirs: [...]` in `oxplow.yaml` — names are single path
segments matched anywhere in the relative path, and apply to both
the workspace watcher and the snapshot store. No changes to
existing snapshots on toggle; newly ignored paths simply stop
appearing in future dirty sets.

### `wiki_note` — `WikiNoteStore` (`src/persistence/wiki-note-store.ts`)

User-curated personal knowledgebase — agent-written writeups, diagrams,
and explanations that accumulate per project. **Bodies live on disk**
as plain markdown files at `.oxplow/notes/<slug>.md` (not committed to
git — this is a personal KB, not team docs). The table only holds
metadata; the filesystem is the source of truth for content.

Columns: `id, slug (UNIQUE), title, captured_head_sha,
captured_refs_json, created_at, updated_at`.

Workflow: the agent writes/edits note files directly with its
Write/Edit tools (no MCP round-trip for bodies). A dedicated watcher
(`src/git/notes-watch.ts`) picks up every file event, re-parses the
file, and upserts metadata. **Every write — agent or user — re-baselines
freshness**, because any write implicitly asserts "this is current as
of now." There is no agent-vs-user distinction.

Freshness is a general indicator, not a proof:
- `captured_head_sha` is HEAD at last write. If HEAD advances, the
  note is flagged `stale`.
- `captured_refs_json` stores `{path, blobSha, mtimeMs}` for every file
  path mentioned in the note (extracted via `parseNoteRefs` in
  `src/persistence/wiki-note-refs.ts`). `computeFreshness` rehashes
  each referenced file; any mismatch → `stale`; any missing file →
  `very-stale`.

MCP tools (`src/mcp/wiki-note-mcp-tools.ts`) are metadata-only —
`list_notes`, `get_note_metadata`, `resync_note`, `search_notes`,
`delete_note`. There is no `create_note` or `update_note`: the agent
Writes the file, then optionally calls `resync_note` to pin freshness
immediately (otherwise the watcher catches up within a ~200ms
debounce).

UI: `NotesPane` (`src/ui/components/Notes/NotesPane.tsx`) is a
left-dock `ToolWindow` — a list of notes with freshness badges and
a `+ New` inline-input row for creating a new slug. Selecting a row
opens the note as a center tab (`note:<slug>`) rendered by
`NoteTab` (`src/ui/components/Notes/NoteTab.tsx`), which owns the
view/edit/delete UI and renders bodies with `react-markdown` +
`remark-gfm` + `mermaid` (mermaid fenced blocks are post-rendered
into inline SVG). IPC surface: `listWikiNotes`, `readWikiNoteBody`,
`writeWikiNoteBody`, `deleteWikiNote`, plus the `wiki-note.changed`
event on the bus.

## The shared `sort_index` queue

The most important invariant in the data model: **`work_items.sort_index`,
`commit_point.sort_index`, and `wait_point.sort_index` all live in the
same numeric space, scoped per thread.**

- `runtime.reorderBatchQueue(streamId, threadId, entries)` rewrites all
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

**Visual vs persistence order for Human Check and Done.** Sections in
`WorkGroupList` render ascending by `sort_index` *except* Human Check
and Done, which render descending (newest-finished items surface on
top). The underlying `sort_index` space is still a single ascending
line — the sections are only flipped at render time. When a
drag-reorder persists a new order, `finalizeReorderIds` in
`plan-utils.ts` reverses each descending run (`human_check`, plus the
`done`/`canceled`/`archived` group) so the `reorderItems` /
`reorderBatchQueue` "sort_index = position" rule produces the intended
visual result. The drag handler passes the *effective* new status of a
row whose status is changing as part of the drop (e.g. a Done row
dropped onto a Human Check row) so the run detector sees the new
section membership. Dropping any item *into* Done is a drop-to-top
contract: the drag handler inserts the row at the head of the Done
bucket in visual order, and `work-item-store.updateItem` bumps
`sort_index` to `MAX+1` on every non-Done → Done transition, so the
two paths agree on "newest-done on top." Any new section with a
non-ascending display must either do the same reversal dance or get
its own flat list.

Constraint: commit and wait points cannot be the very first queue entry —
they have nothing to fire after. Enforced both in the runtime
(`createCommitPoint` / `createWaitPoint` throw if the thread has zero work
items) and in the UI (buttons are disabled with an explanatory tooltip).

## Status diagrams (text)

```
work item:    ready ─► in_progress ─► human_check ─► done ─► archived
                   ╰─────────► blocked
                   ╰─────────► canceled ─► archived

commit point: pending ─► done

wait point:   pending ─► triggered                     (consumed)
```

### Transitions to `in_progress` (server-side guard)

`WorkItemStore.updateItem` rejects any direct jump into `in_progress`
that didn't come from `ready` or `human_check`. The only accepted
sources are:

- `ready → in_progress` (normal pickup)
- `human_check → in_progress` (reopen — the user decided it needs more work)
- `in_progress → in_progress` (no-op)

Everything else (`blocked`, `done`, `canceled`, `archived`) must
transit through `ready` first with an explicit `update_work_item`
call. Rationale: `blocked` is a distinct signal the user parked the
item on; silently promoting it into `in_progress` ignores that
decision. Terminal sources (`done`/`canceled`/`archived`) likewise
require a conscious re-open step. `dispatch_work_item`'s autoStart
path also only fires when the item is currently `ready`.

`listReady` / `readWorkOptions` / `list_ready_work` filter to
`status='ready'` only — `blocked` items are never dispatchable until
un-blocked.

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
- `work-item.changed`, `backlog.changed`, `thread.changed`, `turn.changed`
- `file-snapshot.created`, `agent-status.changed`
- `commit-point.changed`, `wait-point.changed`
- `hook.recorded`, `config.changed`

UI components subscribe via `subscribeOxplowEvents()` (or scoped helpers
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
