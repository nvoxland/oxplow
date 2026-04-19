# Data model

What this doc covers: the SQLite tables, their store classes, and the
single-`sort_index` queue invariant that ties work items, commit points,
and wait points together. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Storage

All persistence lives in one SQLite file under `.newde/state.db`, opened
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

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

### `batches` — `BatchStore` (`src/persistence/batch-store.ts`)

Units of work *within* a stream. Statuses: `active` (writer — may mutate the
worktree), `queued` (read-only, agents can run but writes are denied — see
[agent-model.md](./agent-model.md)'s write-guard section), `completed`
(archived). Exactly one batch per stream is `active`; the others are
`queued` or `completed`. Each batch carries an `agent`-written rolling
`summary` field updated via the `newde__record_batch_summary` MCP tool.

### `work_items` — `WorkItemStore` (`src/persistence/work-item-store.ts`)

The actual TODO list. Kinds: `epic`, `task`, `subtask`, `bug`, `note`.
Statuses: `waiting`, `ready`, `in_progress`, `to_check`, `blocked`, `done`,
`canceled`. `parent_id` chains items under epics. `acceptance_criteria` is
plain text (one criterion per line). Work-item links express dependencies
(`blocks`, `discovered_from`, `relates_to`, …) via the `work_item_links`
join table.

`batch_id` is nullable — items with `batch_id IS NULL` belong to the
**backlog** (a global, stream-less queue). The store API uses the constant
`BACKLOG_SCOPE` as a sentinel string in event payloads so listeners can
distinguish backlog changes from in-batch changes.

### `commit_point` — `CommitPointStore` (`src/persistence/commit-point-store.ts`)

Markers in the queue that say "commit at this point." Mode: `auto` or
`approval`. Status: `pending → proposed → approved → done`, or
`proposed → rejected → pending` on user reject. Holds the agent-proposed
message, the user-approved (possibly edited) message, and the resulting
commit sha. The runtime executes the actual `git commit` — agents only
propose. See [agent-model.md](./agent-model.md) for the Stop-hook flow.

### `wait_point` — `WaitPointStore` (`src/persistence/wait-point-store.ts`)

Markers that pause auto-progression. Status: `pending → triggered`.
Optional `note` shown to the user. Once triggered (the agent stopped at
this point), the marker is "consumed" — the next Stop hook treats it as
past, so prompting the agent at all resumes auto-progression. There is no
"continue" button.

### `agent_turn` — `TurnStore` (`src/persistence/turn-store.ts`)

One row per agent turn (UserPromptSubmit → Stop). Captures the prompt, the
sole-in-progress work item if any, the Claude session id, and an answer
extracted from the batch summary at Stop time. Used by the Activity tab
and by file-change attribution.

### `batch_file_change` — `FileChangeStore` (`src/persistence/file-change-store.ts`)

Per-batch log of file mutations attributed to either a hook (Write/Edit/
MultiEdit/NotebookEdit PostToolUse) or fs-watch (anything else). Carries
turn_id and work_item_id when known. Drives the file-change indicators in
the project pane and the per-turn file filter.

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

Constraint: commit and wait points cannot be the very first queue entry —
they have nothing to fire after. Enforced both in the runtime
(`createCommitPoint` / `createWaitPoint` throw if the batch has zero work
items) and in the UI (buttons are disabled with an explanatory tooltip).

## Status diagrams (text)

```
work item:    waiting ─► ready ─► in_progress ─► to_check ─► done
                   ╰─────────► blocked
                   ╰─────────► canceled

commit point: pending ─► proposed ─► approved ─► done
                              ╰─► rejected ─► pending  (retry loop)

wait point:   pending ─► triggered                     (consumed)
```

## Change events

Every store has a `subscribe(listener)` for in-process listeners. The
runtime relays each store's changes onto the typed EventBus
(`src/core/event-bus.ts`) as `*.changed` events:

- `workspace.changed`, `git-refs.changed`, `workspace-context.changed`
- `work-item.changed`, `backlog.changed`, `batch.changed`, `turn.changed`
- `file-change.recorded`, `agent-status.changed`
- `commit-point.changed`, `wait-point.changed`
- `hook.recorded`, `config.changed`

UI components subscribe via `subscribeNewdeEvents()` (or scoped helpers
like `subscribeWorkspaceEvents`, `subscribeGitRefsEvents`) in
`src/ui/api.ts`. See [ipc-and-stores.md](./ipc-and-stores.md) for how to
plumb a new event end-to-end.

## Related

- [ipc-and-stores.md](./ipc-and-stores.md) — adding new stores and IPC.
- [agent-model.md](./agent-model.md) — how the agent acts on this data.
- [git-integration.md](./git-integration.md) — `gitCommitAll` reads
  approved commit points and writes back the resulting sha.
