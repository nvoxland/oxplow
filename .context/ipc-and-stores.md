# IPC and stores: end-to-end pattern

What this doc covers: the layered flow you follow whenever a feature
needs persistence + IPC + UI, with `commit_point` as a worked example.
For the actual data shapes, see [data-model.md](./data-model.md).

## The 7-layer flow

A new operation that the UI invokes and that mutates persistent state
touches roughly seven files. They sit in this order:

1. **Migration** — `src/persistence/migrations.ts`. Append a new entry to
   `MIGRATIONS` with the next version number. Migrations are append-only;
   never edit a prior entry. `runMigrations` runs them inside a
   transaction and updates `PRAGMA user_version`.

2. **Store class** — `src/persistence/<thing>-store.ts`. Wraps the SQLite
   connection (`getStateDatabase(projectDir)`). Exposes typed read/write
   methods, fires `subscribe()` listeners via the shared `StoreEmitter`
   (`src/persistence/store-emitter.ts`) on changes, validates inputs
   (kinds, statuses, length limits) before writing. Don't reimplement
   the subscribe/emit pattern — instantiate `StoreEmitter<YourChange>`
   in the constructor and delegate.

3. **Runtime method** — `src/electron/runtime.ts`. Adds a method to
   `ElectronRuntime` that resolves stream/thread as needed and delegates
   to the store. Where cross-store atomicity matters, the runtime owns
   that orchestration (e.g. `reorderBatchQueue` updates work_items,
   commit_point, and wait_point in one go).

4. **IPC contract** — `src/electron/ipc-contract.ts`. Add the method
   signature to the `OxplowApi` interface. This is the source of truth for
   what's exposed to the renderer.

5. **Preload binding** — `src/electron/preload.ts`. One line per method:
   `name: (args) => ipcRenderer.invoke("oxplow:name", args)`. Channel
   names follow `oxplow:<methodName>`.

6. **Main-process handler** — `src/electron/main.ts`.
   `handle("oxplow:name", (_event, ...args) => currentRuntime.name(...args))`.
   Use the local `handle()` wrapper, not `ipcMain.handle` directly —
   the wrapper records the channel so `disposeRuntime()` can remove
   every handler before the SQLite database closes (otherwise late
   in-flight requests crash with "database is not open" during quit).

7. **UI api wrapper** — `src/ui/api.ts`. `desktopApi().name(...)` with a
   typed return.

The component then calls the api wrapper and (if the data is reactive)
subscribes to the relevant `*.changed` event to refetch.

## Worked example: `commit_point`

What landed across each layer when commit points were added:

- **Migration v6** in `migrations.ts`: `CREATE TABLE commit_point (...)`,
  plus indexes on `(thread_id, sort_index)` and `(thread_id, status)`.
- **Store**: `src/persistence/commit-point-store.ts` —
  `create/update/markCommitted/delete/setSortIndexes/listForBatch/get`.
  Status machine is `pending → done`; the drafted message lives in
  chat, not the DB. Every mutation calls `emit({ threadId, kind, id })`.
- **Runtime / thread-queue-orchestrator**: methods
  `createCommitPoint/deleteCommitPoint/listCommitPoints/
  reorderBatchQueue/executeCommit` plus the Stop-hook integration via
  `computeStopDirective`. `executeCommit` runs `gitCommitAll` inline
  from the `oxplow__commit` MCP handler.
- **Event**: `CommitPointChangedEvent` added to `src/core/event-bus.ts`,
  published in `runtime.ts` from the store's `subscribe`.
- **IPC contract / preload / main**: `listCommitPoints`,
  `createCommitPoint`, `deleteCommitPoint`. (Approve/reject/setMode/
  reset were removed — approval is chat-driven.)
- **UI api**: `listCommitPoints/createCommitPoint/deleteCommitPoint/
  reorderBatchQueue` in `src/ui/api.ts`.
- **UI consumption**: `PlanPane.tsx` loads commit points on thread change
  and refetches via `subscribeOxplowEvents` filtered to
  `event.type === "commit-point.changed" && event.threadId === threadId`.
- **MCP**: `commit` and `list_commit_points` registered in
  `src/mcp/mcp-tools.ts`'s `buildWorkItemMcpTools` (also added
  `commitPointStore` to its `McpToolDeps`). The agent drafts messages
  in chat and calls `commit` once the user approves; there is no
  `propose_commit` tool — the draft has no DB-side representation.

That's the full template — duplicate the shape for any new persisted
feature.

## Event bus

`src/core/event-bus.ts` defines the typed `OxplowEvent` discriminated
union. To add an event:

1. Add a new interface (`type: "thing.changed"; …`).
2. Add it to the `OxplowEvent` union.
3. Publish from `runtime.ts` inside the relevant store's `subscribe`
   block.
4. Consume in the UI via `subscribeOxplowEvents((e) => { if (e.type === ...) ... })`.

For commonly-filtered events there are scoped helpers in `src/ui/api.ts`:

- `subscribeWorkspaceEvents(streamId, fn)` — filters
  `workspace.changed` by stream.
- `subscribeGitRefsEvents(streamId, fn)` — filters `git-refs.changed`
  by stream.
- `subscribeWorkspaceContext(fn)` — wraps `workspace-context.changed`.

Add a new helper any time more than one component would write the same
filter.

**Listener count:** each UI subscriber adds one listener to
`ipcRenderer`. Electron's default `MaxListeners=10` is too low for
oxplow (~11 stores subscribe on startup), so `src/electron/preload.ts`
calls `ipcRenderer.setMaxListeners(64)` at load time. These are
long-lived per-store subscribers and grow only when we add a store —
not a leak. If the count ever climbs into dozens, switch to a single
preload fan-out bus instead of raising the cap further.

## Cross-store atomicity

When an operation must update multiple tables together, do it in a
runtime method that calls each store's bulk-update API. Stores expose
narrow bulk operations (e.g. `setSortIndexes`) for this; the runtime
isn't allowed to write SQL directly.

The pattern: each store wraps its own writes in a transaction, but
across stores we accept "non-atomic but well-ordered" semantics —
emitting two events (one per store) and letting the UI converge. If you
need stricter atomicity, share a transaction by reaching into
`getStateDatabase` from the runtime and calling each store's prepared
statements inside a single `db.transaction()` block. Don't inline SQL.

## Tests

Each store has a colocated `bun:test` file
(`src/persistence/<thing>-store.test.ts`). Tests use `mkdtempSync` to
spin up a fresh project dir and exercise the public store API. Cross-
store / Stop-hook / MCP behavior goes in
`src/electron/runtime.test.ts`. Run with `bun test <path>`.

Don't mock the DB — every store test hits a real SQLite file. Migrations
are tested in `src/persistence/migrations.test.ts`; if you add a new
migration, add a test that runs it from a clean state and asserts the
expected schema.

## Snapshot store

`SnapshotStore` (`src/persistence/snapshot-store.ts`) is a hybrid: a
SQLite-indexed table (`file_snapshot`) plus an on-disk content-
addressed blob store at `.oxplow/snapshots/objects/xx/yyyy…`. Snapshots
are time-ordered and deduplicated on a `version_hash` (no parent
chain). Rows returned by `listSnapshotsForStream` are pre-enriched
with `label` + `label_kind` joined from `work_item_effort`, and
exclude the first-ever baseline (nothing to diff against). Snapshots
anchor to efforts via `file_snapshot.effort_id` (and the mirror
columns `work_item_effort.start_snapshot_id` /
`end_snapshot_id`). Unlike other stores it doesn't expose a `subscribe()`; the
runtime publishes `file-snapshot.created` on the EventBus after each
successful flush that actually inserted a row.

IPC methods (all go through `ipc-contract.ts` → `main.ts` →
`preload.ts` → `src/ui/api.ts`):

- `listSnapshots(streamId, limit?)` — snapshot rows newest-first,
  baseline excluded, each with `label`/`label_kind`.
- `getSnapshotSummary(snapshotId, previousSnapshotId?)` — snapshot
  row, manifest entries joined with A/M/D kind against the given
  baseline (defaults to the preceding snapshot in time for the
  stream), plus counts.
- `getSnapshotPairDiff(beforeId, afterId, path)` — arbitrary-pair
  diff, used by the Snapshots panel, the Activity tab's per-turn
  view, and the Plan modal's per-effort view.
- `restoreFileFromSnapshot(streamId, snapshotId, path)` — overwrites
  the worktree file with the snapshot's content via the existing
  `writeWorkspaceFile` path (so the UI-echo filter and workspace
  event bus behave the same as a UI edit).
- `listWorkItemEfforts(itemId)` — returns per-effort rows (one per
  `in_progress → human_check` cycle) with pre-joined start/end
  snapshot metadata, linked turn ids, and the changed-paths list
  computed from the pair summary. Used by the Plan modal's Efforts
  section and the "Show in history" jump.

UI subscribe helper: `subscribeSnapshotEvents(streamId, fn)` filters
`file-snapshot.created` by stream and unpacks the payload.

## Transient agent follow-ups

`FollowupStore` (`src/electron/followup-store.ts`) is a pure in-memory
map keyed by `threadId`. It backs three orchestrator-only MCP tools —
`oxplow__add_followup`, `oxplow__remove_followup`,
`oxplow__list_followups` — and lets the agent stash a "I'll get back to
that next" reminder mid-turn without filing a durable work item. No
SQLite involvement, no migration, lost on runtime restart.

Surfaces:

- The store exposes `add/remove/list/clear/subscribe`. The runtime
  re-publishes its `subscribe` events as `followup.changed`
  (`{ threadId, kind: "added" | "removed" | "cleared", id }`) on the
  EventBus.
- `getThreadWorkState` (the main IPC for the Work panel) layers the
  thread's current followups onto its response inside the
  `followups` field, so PlanPane / WorkGroupList see them alongside
  durable work items without a second round-trip. The work-item-api
  wrapper owns that overlay; the persistence-layer
  `WorkItemStore.getState` always returns `followups: []`.
- IPC: only one new method — `removeFollowup(threadId, id)` — used by
  the ✕ dismiss button on each follow-up row. Adds happen
  exclusively via the MCP tool surface; the UI never adds.
- App.tsx subscribes to `followup.changed` and re-fetches
  `getThreadWorkState` for the affected thread (stream id is recovered
  from the cached `threadStates` map).

Rendering: `WorkGroupList.tsx` renders each follow-up as an italic
muted "↳ follow-up: <note>" line at the very top of the To Do section
(only on the root group, never on epic-children panes), with a single
✕ dismiss button. No status icon, no drag, no context menu.

When to use a follow-up vs. a task: see the agent skill at
`.oxplow/runtime/claude-plugin/skills/oxplow-runtime/SKILL.md`. Rule:
if the deferred ask warrants a row the user reviews/accepts, file a
task; if it's just a within-conversation bookmark, add a follow-up
and remove it in the same turn you handle it. Never carry both.

## Work panel in_progress bucket is task-only

The Work panel's in_progress bucket is driven purely by `work_item`
rows (`status = 'in_progress'` for the active thread). There are no
synthesized turn rows, no live-prompt overlay, and no IPC for
listing open turns — `listAgentTurns`, `listOpenTurns`,
`listRecentInactiveTurns`, `archiveAgentTurn`, and
`subscribeTurnEvents` no longer exist, and there is no
`TurnChangedEvent`. If you need a "what is the agent doing right
now" signal, use the `work_item` rows themselves plus
`agent-status.changed` for the colored-dot working/waiting/idle
state.

## Thread and stream reorder IPC

- `reorderBatches(streamId, orderedBatchIds[])` — reassigns sequential
  `sort_index` values to the given thread ids (only rows belonging to
  `streamId` are updated). Emits `thread.changed` (kind: "reordered").
  Promoting or completing a thread no longer auto-moves it to position 0;
  the user controls order via drag-to-reorder in `BatchRail`.
- `reorderStreams(orderedStreamIds[])` — reassigns sequential
  `sort_index` to streams. Emits `stream.changed` (kind: "reordered").
  `listStreams` now orders by `sort_index, rowid` instead of
  `created_at, rowid`.
- `getWorkNotes(itemId)` — returns `WorkNote[]` sorted by `created_at ASC`
  for the given work item. Read-only; called when the edit modal opens to
  populate the read-only Notes section. No stream/thread context needed.

Both follow the standard 7-layer IPC flow (migration → store →
runtime → ipc-contract → preload → main → ui/api).

## Related

- [data-model.md](./data-model.md) — the actual schemas.
- [agent-model.md](./agent-model.md) — how the agent calls into MCP
  tools that wrap these stores.
