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
   `ElectronRuntime` that resolves stream/batch as needed and delegates
   to the store. Where cross-store atomicity matters, the runtime owns
   that orchestration (e.g. `reorderBatchQueue` updates work_items,
   commit_point, and wait_point in one go).

4. **IPC contract** — `src/electron/ipc-contract.ts`. Add the method
   signature to the `NewdeApi` interface. This is the source of truth for
   what's exposed to the renderer.

5. **Preload binding** — `src/electron/preload.ts`. One line per method:
   `name: (args) => ipcRenderer.invoke("newde:name", args)`. Channel
   names follow `newde:<methodName>`.

6. **Main-process handler** — `src/electron/main.ts`.
   `handle("newde:name", (_event, ...args) => currentRuntime.name(...args))`.
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
  plus indexes on `(batch_id, sort_index)` and `(batch_id, status)`.
- **Store**: `src/persistence/commit-point-store.ts` —
  `create/setMode/propose/approve/reject/markDone/resetToPending/delete/
  setSortIndexes/listForBatch/listApproved/get`. Every mutation calls
  `emit({ batchId, kind, id })`.
- **Runtime**: `commitPointStore` field, methods
  `createCommitPoint/setCommitPointMode/approveCommitPoint/
  rejectCommitPoint/resetCommitPoint/deleteCommitPoint/listCommitPoints/
  reorderBatchQueue` (cross-store) plus the internal
  `findActiveCommitPoint/executeApprovedCommit` helpers and the
  Stop-hook integration via `computeStopDirective`.
- **Event**: `CommitPointChangedEvent` added to `src/core/event-bus.ts`,
  published in `runtime.ts` from the store's `subscribe`.
- **IPC contract / preload / main**: one line each for every public
  method.
- **UI api**: `listCommitPoints/createCommitPoint/setCommitPointMode/
  approveCommitPoint/rejectCommitPoint/resetCommitPoint/deleteCommitPoint/
  reorderBatchQueue` in `src/ui/api.ts`.
- **UI consumption**: `PlanPane.tsx` loads commit points on batch change
  and refetches via `subscribeNewdeEvents` filtered to
  `event.type === "commit-point.changed" && event.batchId === batchId`.
- **MCP**: `propose_commit` and `list_commit_points` registered in
  `src/mcp/mcp-tools.ts`'s `buildWorkItemMcpTools` (also added
  `commitPointStore` to its `McpToolDeps`).

That's the full template — duplicate the shape for any new persisted
feature.

## Event bus

`src/core/event-bus.ts` defines the typed `NewdeEvent` discriminated
union. To add an event:

1. Add a new interface (`type: "thing.changed"; …`).
2. Add it to the `NewdeEvent` union.
3. Publish from `runtime.ts` inside the relevant store's `subscribe`
   block.
4. Consume in the UI via `subscribeNewdeEvents((e) => { if (e.type === ...) ... })`.

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
newde (~11 stores subscribe on startup), so `src/electron/preload.ts`
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
addressed blob store at `.newde/snapshots/` (`objects/xx/yyyy…` +
`manifests/<id>.json`). Unlike other stores it doesn't expose a
`subscribe()`; the runtime publishes `file-snapshot.created` on the
EventBus after each successful flush.

IPC methods (all go through `ipc-contract.ts` → `main.ts` →
`preload.ts` → `src/ui/api.ts`):

- `getTurnFileDiff(turnId, path)` — before/after for a path within a
  single turn.
- `listSnapshots(streamId, limit?)` — snapshot rows newest-first.
- `getSnapshotSummary(snapshotId)` — snapshot row, manifest entries
  joined with A/M/D kind against the parent chain, plus counts.
- `getSnapshotFileDiff(snapshotId, path)` — before/after for a path
  between a snapshot and its parent.
- `getSnapshotPairDiff(beforeId, afterId, path)` — arbitrary-pair
  diff, used by the Snapshots panel's compare mode.
- `restoreFileFromSnapshot(streamId, snapshotId, path)` — overwrites
  the worktree file with the snapshot's content via the existing
  `writeWorkspaceFile` path (so the UI-echo filter and file-change
  event bus behave the same as a UI edit).

UI subscribe helper: `subscribeSnapshotEvents(streamId, fn)` filters
`file-snapshot.created` by stream and unpacks the payload.

## Related

- [data-model.md](./data-model.md) — the actual schemas.
- [agent-model.md](./agent-model.md) — how the agent calls into MCP
  tools that wrap these stores.
