# IPC and stores: end-to-end pattern


What this doc covers: the layered flow you follow whenever a feature
needs persistence + IPC + UI, with `wiki_page` as a worked example.
For the actual data shapes, see [data-model.md](./data-model.md).

## The 7-layer flow

A new operation that the UI invokes and that mutates persistent state
touches roughly seven files. They sit in this order:

1. **Migration** — `crates/oxplow-db/migrations/V1__initial_schema.sql`. Append a new entry to
   `MIGRATIONS` with the next version number. Migrations are append-only;
   never edit a prior entry. `runMigrations` runs them inside a
   transaction and updates `PRAGMA user_version`.

2. **Store class** — `crates/oxplow-db/src/<thing>_store.rs`. Wraps a
   `Database` handle (`Database::open(...)`). Exposes typed read/write
   methods on the relevant `*Store` trait from `oxplow-domain`,
   validates inputs (kinds, statuses, length limits) before writing.
   Cross-store change fan-out goes through the shared `EventBus`
   (`crates/oxplow-app/src/events.rs`); stores call `events.emit(...)`
   with the matching `OxplowEvent` variant rather than maintaining
   their own subscriber list.

3. **Runtime method** — `crates/oxplow-app/src/lib.rs`. Adds a method to
   `Services` that resolves stream/thread as needed and delegates
   to the store. Where cross-store atomicity matters, the runtime owns
   that orchestration.

4. **IPC contract** — `crates/oxplow-tauri-ipc/src/commands/`. Add the method
   signature to the `OxplowApi` interface. This is the source of truth for
   what's exposed to the renderer.

5. **Tauri command + binding** — add a `#[tauri::command] #[specta::specta]`
   adapter alongside the others in
   `crates/oxplow-tauri-ipc/src/commands/<area>.rs` and register it in
   `specta_builder()` (`crates/oxplow-tauri-ipc/src/lib.rs`). The
   `cargo test -p oxplow-tauri-ipc` `export_ts_bindings` test
   regenerates `apps/desktop/src/tauri-bridge/generated/bindings.ts`;
   CI fails on a non-empty diff after regen, so check that file in.

   **Shared dispatch (`oxplow-rpc`)**: the command *body* lives as a
   plain core fn in `crates/oxplow-rpc/src/commands.rs`
   (`async fn(svc: &Services, args…) -> Result<T, IpcError>`), and the
   Tauri adapter is a one-line delegate to it. Register the core in the
   `rpc_dispatch!` registry (`crates/oxplow-rpc/src/lib.rs`) under the
   same snake_case wire name; the registry powers the remote-daemon
   HTTP path (`POST /ipc/:name`), so a command that exists only as a
   Tauri adapter is invisible to remote mode. `IpcError` (and every
   `From<…> for IpcError` impl) lives in `oxplow-rpc::error` —
   `oxplow-tauri-ipc::error` is a re-export. `oxplow-rpc` must stay
   free of `tauri` deps so the headless daemon builds without a
   webview toolchain. Commands that touch the OS shell
   (`AppHandle`/window/clipboard/menu) stay Tauri-only and are NOT
   registered in the dispatch.

6. **UI api wrapper** — `apps/desktop/src/api.ts`. Thin wrapper around
   the typed `commands.name(...)` import from the generated bindings.

   **Transport switch**: the generated bindings import `invoke` from
   `apps/desktop/src/tauri-bridge/transport.ts` (the export test
   rewrites the import line post-export), and all event subscriptions
   import `listen` from the same module. It routes **per command**:
   shell commands (the ones step 5 says stay Tauri-only — they're
   listed in `oxplow_tauri_ipc::SHELL_ONLY_COMMANDS` and generated into
   `tauri-bridge/generated/shellCommands.ts`) go to `@tauri-apps/api`;
   everything else goes to this window's daemon as `POST
   <base>/ipc/<name>`, with events off the multiplexed `/events`
   WebSocket. The daemon base is per window — see
   [remote-daemon.md](./remote-daemon.md). Never import
   `@tauri-apps/api/core` or `/event` directly from UI code.

   **If you add a Tauri-only command, add it to `SHELL_ONLY_COMMANDS`.**
   Otherwise the transport sends it to the daemon, which 404s — and the
   surface-parity test will tell you so.

The component then calls the api wrapper and (if the data is reactive)
subscribes to the relevant `*.changed` event to refetch.

## Worked example: `wiki_page`

Concrete instance of the 7-layer flow. Look at the `wiki_page` table,
`WikiPageStore`, the runtime's `listWikiPages`/`writeWikiPageBody`
helpers, the matching IPC contract entries, the preload bindings, the
main-process handlers, and the UI api wrappers. Every other persisted
feature in this codebase follows the same shape — duplicate it for new
work.

## Event bus

`crates/oxplow-app/src/events.rs` defines the typed `OxplowEvent` discriminated
union. To add an event:

1. Add a variant to the `OxplowEvent` enum. The wire format uses
   `#[serde(tag = "kind", rename_all = "camelCase")]`, so a Rust
   variant `FooChanged { stream_id }` lands on the wire as
   `{ kind: "fooChanged", streamId }`.
2. Re-run `cargo test -p oxplow-tauri-ipc` to regenerate the TS
   bindings. `OxplowEvent` is exported through the specta builder, and
   `OxplowEventKind` in `apps/desktop/src/tauri-bridge/index.ts` is
   derived from the generated union (`OxplowEvent["kind"]`) — there is
   no hand-maintained kind list anymore. CI's bindings-drift guard
   fails the PR if you forget the regen.
3. Publish from the relevant service or command by calling
   `state.events.emit(OxplowEvent::FooChanged { … })`. The Tauri shell
   forwards every emit to the renderer via
   `app_handle.emit(event_channels::OXPLOW, ...)`.
4. Consume in the UI via
   `subscribeOxplowEvents((e) => { if (e.kind === "fooChanged") … })`.

**Camelcase trap (mostly defused):** the wire shape is camelCase
(`event.kind`, `event.streamId`, …). A subscriber that filters on
`event.type === "foo.changed"` will silently never fire — the
agent-status dot bug came from exactly this mismatch. The kind union
is now generated, so the remaining trap is only in hand-written
filter strings at subscriber call sites.

**Event channel names** live in one registry:
`oxplow_app::event_channels` (`crates/oxplow-app/src/events.rs`) on
the Rust side — consumed by the Tauri shell's emit bridges and the
daemon's `/events` frame keys — mirrored by
`apps/desktop/src/tauri-bridge/channels.ts` (`EVENT_CHANNELS`) on the
renderer side. The surface-parity test
(`event_channels_match_typescript`) fails if the two diverge; update
both together when adding a channel.

## Git dashboard / cross-worktree IPC

The Git Dashboard page added five renderer-callable methods to
`DesktopApi`. Each delegates to a helper in `crates/oxplow-git/src/lib.rs` after
resolving the stream's `worktree_path` (the same pattern as
`getGitLog`):

- `getAheadBehind(streamId, base, head?)` — `{ ahead, behind }` for
  the branch header / worktree rows.
- `getCommitsAheadOf(streamId, base, head, limit?)` —
  `GitLogCommit[]` for pairwise commit-diff displays.
- `listRecentRemoteBranches(streamId, limit?)` —
  `RemoteBranchEntry[]` sorted by committer date.
- `gitPushCurrentTo(streamId, remote, branch)` — refspec push of
  HEAD into `<remote>/<branch>`. Wraps the async git helper and opens
  a `BackgroundTaskStore` row.
- `gitPullRemoteIntoCurrent(streamId, remote, branch)` — fetch +
  merge, also background-task wrapped.
- `listSiblingWorktrees(streamId)` — every git worktree of this repo
  except the one backing `streamId`. Used by the dashboard's
  worktrees card. Distinct from `listAdoptableWorktrees`, which
  returns only worktrees NOT yet tracked as oxplow streams (used by
  the new-stream adoption flow).

The cross-worktree merge action reuses the existing `gitMergeInto`
IPC method; no new method is needed because merging only ever runs in
the *current* stream's working dir. See
[git-integration.md](./git-integration.md) for the rationale on why
no symmetric "push commits into another worktree" IPC exists.

For commonly-filtered events there are scoped helpers in `apps/desktop/src/api.ts`:

- `subscribeWorkspaceEvents(streamId, fn)` — filters
  `workspace.changed` by stream.
- `subscribeGitRefsEvents(streamId, fn)` — filters `git-refs.changed`
  by stream.
- `subscribeWorkspaceContext(fn)` — wraps `workspace-context.changed`.

Add a new helper any time more than one component would write the same
filter.

Config IPC includes `set_agents(agents: Vec<AgentKind>)`, which writes the
project's ordered enabled-agent list in `.oxplow/project.yaml`. Thread creation accepts
an optional `agent`; the command validates that the requested agent is enabled
and otherwise uses the first configured agent.

**Listener count:** each UI subscriber registers via
`listen("oxplow:event", ...)` from `@tauri-apps/api/event`. Tauri 2's
event bus has no `MaxListeners` cap, so the historical
`setMaxListeners(64)` workaround is gone. Subscribers are still
long-lived per-store and grow only when we add a store; if you find
yourself adding many short-lived listeners, prefer a single fan-out
helper in `apps/desktop/src/api.ts` rather than calling `listen`
directly from each component.

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

Each store has Rust unit tests inline at the bottom of its source
file (`crates/oxplow-db/src/<thing>_store.rs`, `#[cfg(test)] mod
tests`). Tests use `Database::in_memory()` for an in-process SQLite
or a `tempfile::tempdir()` for file-backed cases. Cross-store /
Stop-hook / MCP behavior lives in `crates/oxplow-app/src/lib.rs`'s
`#[cfg(test)] mod tests` and the integration tests under
`crates/oxplow-app/tests/`. Run with `cargo test -p <crate>` or
`cargo test --workspace`.

Don't mock the DB — every store test hits a real SQLite handle.
Migrations are tested alongside `Database::open` in
`crates/oxplow-db/src/database.rs`; if you add a new migration,
add a test that runs it from a clean state and asserts the expected
schema.

## Snapshot store

`SnapshotStore` (`crates/oxplow-db/src/analytics_stores.rs`) is a hybrid: a
SQLite-indexed table (`file_snapshot`) plus an on-disk content-
addressed blob store at `.oxplow/snapshots/objects/xx/yyyy…`. Snapshots
are time-ordered and deduplicated on a `version_hash` (no parent
chain). Rows returned by `listSnapshotsForStream` are pre-enriched
with `label` + `label_kind` joined from `task_effort`, and
exclude the first-ever baseline (nothing to diff against). Snapshots
anchor to efforts via `file_snapshot.effort_id` (and the mirror
columns `task_effort.start_snapshot_id` /
`end_snapshot_id`). Unlike other stores it doesn't expose a `subscribe()`; the
runtime publishes `file-snapshot.created` on the EventBus after each
successful flush that actually inserted a row.

IPC methods (all go through `ipc-contract.ts` → `main.ts` →
`preload.ts` → `apps/desktop/src/api.ts`):

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
- `listTaskEfforts(itemId)` — returns per-effort rows (one per
  `in_progress → human_check` cycle) with pre-joined start/end
  snapshot metadata, linked turn ids, and the changed-paths list
  computed from the pair summary. Used by the Plan modal's Efforts
  section and the "Show in history" jump.

UI subscribe helper: `subscribeSnapshotEvents(streamId, fn)` filters
`file-snapshot.created` by stream and unpacks the payload.

## Transient agent follow-ups

`FollowupStore` (`crates/oxplow-app/src/followup.rs`) is a pure in-memory
map keyed by `threadId`. It backs three orchestrator-only MCP tools —
`oxplow__add_followup`, `oxplow__remove_followup`,
`oxplow__list_followups` — and lets the agent stash a "I'll get back to
that next" reminder mid-turn without filing a durable task. No
SQLite involvement, no migration, lost on runtime restart.

Surfaces:

- The store exposes `add/remove/list/clear/subscribe`. The runtime
  re-publishes its `subscribe` events as `followup.changed`
  (`{ threadId, kind: "added" | "removed" | "cleared", id }`) on the
  EventBus.
- `getThreadWorkState` (the main IPC for the Work panel) layers the
  thread's current followups onto its response inside the
  `followups` field, so PlanPane / WorkGroupList see them alongside
  durable tasks without a second round-trip. The task-api
  wrapper owns that overlay; the persistence-layer
  `taskstore.getState` always returns `followups: []`.
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

## Background tasks (long-running op progress)

`BackgroundTaskStore` (`crates/oxplow-app/src/background_task.rs`) is
another in-memory store, modeled on `FollowupStore`. It surfaces "what
is the runtime doing right now" rows in the bottom-bar
`BackgroundTaskIndicator` (`apps/desktop/src/components/`). No SQLite, no
migration, lost on restart. Done/failed rows linger for a 4s grace
window so the UI can flash a checkmark before evicting.

Producers call `start({ kind, label, detail?, progress? })` to register
a row, optionally `update(id, patch)` for progress ticks, then
`complete(id)` or `fail(id, message)`. `progress` is `0..1` for
determinate work or `null` for indeterminate (animated stripes in the
UI). Active producers:

- **Git pull/push/merge/rebase** — `runtime.gitPull` /
  `runtime.gitPush` / `runtime.gitMergeInto` / `runtime.gitRebaseOnto`
  use `gitPullAsync` / `gitPushAsync` / `gitMergeAsync` /
  `gitRebaseAsync` from `crates/oxplow-git/src/lib.rs` so the main process doesn't
  block during the network or merge work. Indeterminate.
- **Code-quality scans** — `runtime.runCodeQualityScan` opens a row in
  parallel with the existing `code-quality.scanned` event flow. The
  scan-status strip in CodeQualityPanel keeps its panel-local spinner;
  the bottom-bar row is the global indicator. Indeterminate.
- **LSP cold start** — `LspSessionManager` (`crates/oxplow-app/src/lsp_sessions.rs` over `crates/oxplow-lsp/src/proxy.rs`) takes
  optional `onInitializeStart` / `onInitializeEnd` hooks. The runtime
  wires them to `start`/`complete`. Indeterminate.
- **LSP install** — `crates/oxplow-app/src/lsp_installer.rs` wraps
  `crates/oxplow-lsp-installer/` (Mason-registry-backed). `install_lsp_package`
  IPC downloads a release asset, drops it under `.oxplow/lsp/<name>/`,
  and registers the binary with `LspSessionManager`'s
  `InstalledServers` overlay. Manifest at `.oxplow/lsp/installed.json`
  replays into the session manager on boot. `remove_lsp_package`
  reverses all of that. Both emit `LspServersChanged` on the oxplow
  event bus so the renderer refetches `list_lsp_servers`. The shared
  session surface (`lsp_request` / `lsp_notify` / `list_lsp_servers` /
  `restart_lsp_server` / `respond_lsp_apply_edit`) is documented in
  `.context/lsp.md`.
- **Notes wiki resync** — `NotesWatcher.start` (`crates/oxplow-fs-watch/src/lib.rs`)
  takes `onScanStart` / `onScanProgress` / `onScanEnd` callbacks. The
  runtime registers a row only when `total >= 5` (smaller dirs aren't
  worth a flash). Determinate.

IPC: one method `listBackgroundTasks()` returns the snapshot. Renderer
subscribes via `subscribeBackgroundTaskEvents(onChange)` (filters
`background-task.changed` events) and refetches. The renderer never
writes — only the runtime starts/updates tasks. Cancellation is not
supported (v1).

`get(id)` falls back to a longer-retention **snapshot map** when the
live row has been evicted. The snapshot is captured on
`complete()`/`fail()` and retained for `SNAPSHOT_RETENTION_MS` (5 min,
LRU-capped) so the renderer's `awaitBackgroundTask` can still read the
final `result` / `error` even if the 4s grace window expired between
the "ended" event and the IPC re-fetch. Without this, fast git ops
that succeed silently could surface a blank op-error page (no stderr,
no stdout, no exitCode) — see the diagnostics fields on `GitOpResult`
(`args`, `projectDir`, `durationMs`, `signal`, `blankFailure`) which
flow into `OpError` and the OpErrorPage when something does fail.

Adding a new producer: don't widen the union — extend
`BackgroundTaskKind` and pick the most relevant existing kind, or add
one in `background-task-store.ts` plus a label entry in
`BackgroundTaskIndicator.tsx` (`KIND_LABEL`). Wire `start`/`complete`
or `fail` in the new spot; events publish automatically.

## Work panel in_progress bucket is task-only

The Work panel's in_progress bucket is driven purely by `task`
rows (`status = 'in_progress'` for the active thread). There are no
synthesized turn rows, no live-prompt overlay, and no IPC for
listing open turns — `listAgentTurns`, `listOpenTurns`,
`listRecentInactiveTurns`, `archiveAgentTurn`, and
`subscribeTurnEvents` no longer exist, and there is no
`TurnChangedEvent`. If you need a "what is the agent doing right
now" signal, use the `task` rows themselves plus
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
Both follow the standard 7-layer IPC flow (migration → store →
runtime → ipc-contract → preload → main → ui/api).

## Generic usage tracking

`UsageStore` (`crates/oxplow-db/src/analytics_stores.rs`) records `(kind, key,
event)` rows with optional `stream_id` and a 30s coalesce window. The
runtime exposes four read methods + one writer over IPC:

- `recordUsage({ kind, key, event?, streamId?, threadId? })`
- `listRecentUsage({ kind, streamId?, threadId?, limit?, since? })` —
  most-recent keys aggregated by `key`, returning
  `{ key, last_at, count }[]`. Pass `streamId` to scope to one
  workspace, `threadId` to scope to one thread, or both to
  intersect.
- `listFrequentUsage({ kind, streamId?, threadId?, limit?, since? })`
  — same shape ranked by count.
- `listCurrentlyOpenUsage({ kind, streamId?, threadId? })` — keys
  whose latest event is `"open"` with no later `"close"`. Returns
  `[]` for kinds that don't emit close events yet.

Subscribe in the UI via `subscribeUsageEvents(fn, { kind })`. The bus
event is `usage.recorded` with `{ kind, key, streamId, threadId }`.

Active write hookpoints (all in `apps/desktop/src/App.tsx`, all pass both
`streamId` and the active `threadId` so per-stream and per-thread
queries both work):

- `wiki-note` → `handleOpenNote`
- `editor-file` → `handleOpenFile`
- `task` → `handleRequestEditTask`

The wiki-note rows currently feed `NotesPane`'s "Recently visited"
section; editor-file and task rows are recorded but not yet
surfaced in any UI — the architecture is ready, the consumer is the
follow-up.

When wiring a new kind, record the visit at the moment the user
*opens* the target — not on hover or focus — because the 30s coalesce
relies on `occurred_at` being a real "user intent to read this"
signal.

## `PageVisitStore` — page navigation event log

Distinct from `UsageStore` (which is a generic kind/key tracker with
coalescing). `PageVisitStore` records every `App.handleOpenPage` call
as a fresh event with the full `TabRef` (kind + id + payload + label).
Append-only, no coalescing — analytics queries collapse as needed.

IPC:

- `recordPageVisit(input)` — called from `handleOpenPage` for every
  ref except `agent`/`new-stream`/`new-task`. The label is
  resolved from richer context first (task title) and falls back
  to `deriveDefaultLabel(ref)` from
  `apps/desktop/src/components/RailHud/history.ts`.
- `listRecentPageVisits({threadId,limit,dedupeByRef,excludeKinds})`
  — newest-first; with `dedupeByRef` collapses to one row per
  `ref_id`. Drives the rail's History section.
- `topVisitedPages({threadId,sinceT,limit,excludeKinds})` — count
  rollup with the latest payload+label per ref. Drives the rail's
  "Most visited" toggle and the Visits dashboard.
- `countPageVisitsByDay({refId,threadId,sinceT,untilT})` — daily
  bucketed counts for behavior-over-time charts.
- `forgetPage(refKind, refId)` — wipes every visit row for a page
  reference and emits `page-visit.changed`. Used when a page is
  deleted (real persistent or virtual, e.g. an op-error entry being
  dismissed/cleared) so it disappears from rail history. Generic — not
  tied to any one page kind.

Subscribe via `subscribePageVisitEvents(fn)`; bus event is
`page-visit.changed` with `{ refId, refKind, threadId }`. The same
event fires on `forget`, with `threadId: null` (the row spans threads).

## Code quality scans

`CodeQualityStore` (`crates/oxplow-db/src/analytics_stores.rs`) is
another straightforward 7-layer instance, plus a `crates/oxplow-app/src/`
module (the first user) that wraps the external CLIs. The runtime
method `runCodeQualityScan` shows the canonical pattern for "store
+ subprocess + git-diff scoping":

1. `store.startScan(...)` → `code-quality.scanned` event with
   `status: "running"` fires immediately so the UI can show a
   spinner before the subprocess settles.
2. Resolve the stream's worktree, optionally call
   `listBranchChanges(worktree, baseRef)` for `scope: "diff"` and
   pass the changed-files list into the subprocess function.
3. `runLizard` / `runJscpd` return *normalized* findings
   (`{ path, startLine, endLine, kind, metricValue, extra }`); the
   runtime never sees tool-specific shapes.
4. On success, `store.completeScan(scanId, findings)` inserts
   findings, flips status, and prunes old scans for the same
   `(stream, tool, scope)` triple — all in one transaction.
5. On `CodeQualityToolMissingError` (ENOENT), `failScan` records a
   user-friendly "tool not installed" message that the UI surfaces
   in its scan-status strip.

When adding a third CLI tool, you only touch the subprocess module
(new `runFoo` + parser) and the `CodeQualityTool` union — the
store, runtime, IPC, and UI are tool-agnostic.

## Collection observations

The collection subsystem (`.context/collection.md`) follows the same
7-layer shape with a couple of slice patterns worth calling out:

- **One read IPC, one event.** `list_effort_observations(effortId,
  kind?)` (`crates/oxplow-tauri-ipc/src/commands/effort.rs`, over
  `Services.observation_store`) is `Both` in the surface-parity manifest
  — same name on IPC and MCP. Mutations are agent-only (MCP
  `ingest_coverage` / `record_test_run`) since the UI never writes
  observations. The renderer refetches on `EffortObservationsChanged
  { threadId, effortId }` — wire kind `effortObservationsChanged`
  (mirrored in `apps/desktop/src/tauri-bridge/index.ts`'s
  `OxplowEventKind`; mind the camelcase trap above). `TaskPage` →
  `EffortObservations` subscribes and refetches per effort.
- **The engine lives in `oxplow-app`, not a store.** `CollectionService`
  (`crates/oxplow-app/src/collection.rs`) owns the orchestration
  (effort resolution, coverage parse via `oxplow-coverage`, changed-line
  diff, freshness pin) and is called from both the MCP tools and the
  control-plane PostToolUse hook. The store
  (`SqliteEffortObservationStore`) stays a thin typed read/write surface.

## Agent nudges

The persisted record of the informational nudges `CollectionService`
surfaces to the agent from the PostToolUse hook (report-less-run +
coverage-target). A standard 7-layer instance backed by
`SqliteAgentNudgeStore` (`crates/oxplow-db/src/agent_nudge_store.rs`;
schema in [data-model.md](./data-model.md), migration `V33`).

- **IPC** (UI-only — the agent never reads nudges back): two read methods,
  `list_nudges_for_effort(effortId)` and `list_nudges_for_thread(threadId)`
  (`crates/oxplow-rpc/src/commands/effort.rs`, adapters in
  `crates/oxplow-tauri-ipc/src/commands/effort.rs`, registered in the
  `rpc_dispatch!` registry and the surface-parity manifest as `ui()`).
  There are no write IPCs — nudges are written exclusively by the service
  inside `on_post_tool_use`, best-effort (a persistence error is logged,
  never fails the hook).
- **Event**: `AgentNudgesChanged { threadId, effortId: Option<String> }`
  (wire kind `agentNudgesChanged`) emitted by the service after a successful
  record. The renderer's collapsed "Agent nudges" debug sub-view
  (`EffortObservations.tsx` → `AgentNudgesBlock`) subscribes and refetches
  per effort, alongside the coverage/tests block. Persistence happens AFTER
  the existing one-shot dedup gates, so a deduped nudge is never stored or
  re-emitted.

## Multi-owner stores: the `page_ref` slice pattern

Most stores have a single writer per row. The unified
cross-page-reference graph (`page_ref` table; see
[data-model.md](./data-model.md)) is the exception: a single
`(source_kind, source_id)` like `(task, wi-42)` accumulates
rows from three different writers (the task store's body
mentions, the link store's `task_link:*` edges, the effort
store's `touched_file` edges), each owning a slice keyed by
`ref_type`.

The pattern that lets writers co-own a source without trampling
each other:

1. **Pure projections** (`crates/oxplow-db/src/page_ref_projections.rs`)
   turn each writer's domain rows into `Vec<PageRefEdge>`. Each
   helper also exposes a small list of the `ref_type`s it owns
   (`task_body_ref_types()`, `task_link_ref_types()`,
   `effort_ref_types()`).
2. **Slice-replace** at the store
   (`SqlitePageRefStore::replace_source_for_ref_types`) takes
   `(source_kind, source_id, ref_types, edges)` and atomically
   `DELETE`s only rows matching the source AND one of the
   ref_types, then inserts the new edges. Other slices for the
   same source survive untouched.
3. **Built-in projection** at each writer store. Each owning
   `Sqlite*Store` constructs its own `SqlitePageRefStore` over the
   same `Database` in `new()` (the field is NOT optional), and the
   relevant write methods (upsert, record_file, link create/delete)
   call the slice helper after the primary write. There is no way to
   construct a store that silently skips graph mirroring — tests get
   the projection for free, and the backfill
   (`page_ref_backfill.rs`) exists only for rows written before
   mirroring did.

When a single writer owns the WHOLE source (wiki sync, findings
write, commit indexer), use the simpler `replace_source` instead.

To add a new source kind to the graph: add a projection helper, a
ref-type-list helper, construct the page-ref mirror inside the owning
store's `new()`, and call the slice or full replace from its write
methods.
For body-text sources that should pick up the same wikilink rules
the wiki + tasks use, route through
`oxplow_domain::refs::extract` rather than re-implementing the
parser.

## Exception: commands with no `Services` (launcher)

Not every command follows the 7-layer flow. The launcher / multi-window
commands in `crates/oxplow-tauri-ipc/src/commands/launch.rs`
(`get_launch_mode`, `list_recent_projects`, `remove_recent_project`,
`open_project`) deliberately depend on **neither** a SQLite store
**nor** `AppState` (`Services`) — they must work in launcher mode where
no project is booted. Instead they read managed state
`RecentProjectsState` (`Arc<oxplow_config::RecentProjects>`, a global
JSON file) and `LaunchInfo`, both managed by `main.rs` in *both* launch
modes. `open_project` spawns a new process rather than mutating any
store. When you add a command the launcher window needs, keep it on
this no-`Services` footing; everything else uses the flow above.

## Comments

Threaded annotations anchored to a text selection on any page — a
straightforward 7-layer instance (`SqliteCommentStore`,
`crates/oxplow-db/src/comment_store.rs`; schema in
[data-model.md](./data-model.md)). The Tauri commands live in
`crates/oxplow-tauri-ipc/src/commands/comments.rs`: `create_comment`,
`add_comment_message`, `list_comments_for_target`,
`list_comments_for_stream`, `set_comment_intent`, `set_comment_status`,
`set_comment_anchor`, `delete_comment`. The same store is exposed to the
agent via three MCP tools (`list_comments`, `respond_to_comment`,
`resolve_comment`) — see [agent-model.md](./agent-model.md).

`create_comment` takes a single `CreateCommentRequest` struct argument
(not positional params) because the field count — `stream_id`,
`thread_id`, `target_kind`/`target_id`, `quote`, `selectors_json` (the
W3C selectors array, renamed from `anchor_json` in V24), `context_chain`
+ `referenced_refs` (`Vec<CommentTarget>` typed context), `intent`,
`author`, `body` — exceeds tauri-specta's 10-argument cap on a
`#[tauri::command]`. This mirrors the `CreateTaskRequest` pattern in
`commands/tasks.rs`: when a command would take too many args, bundle them
in a `#[derive(Serialize, Deserialize, Type)] #[serde(rename_all =
"camelCase")]` request struct. `set_comment_anchor` / `relink_comment`
take `selectors_json` (same rename).

Every mutating command (and the MCP `respond_to_comment` /
`resolve_comment`) emits `OxplowEvent::CommentsChanged { streamId,
targetKind, targetId }`; the wire kind is `commentsChanged`, mirrored in
`OxplowEventKind` in `apps/desktop/src/tauri-bridge/index.ts`. The
renderer subscribes to refetch the affected page's comments + the
Comments inbox. `set_comment_anchor` is the one mutation that does **not**
emit — it's a passive re-anchor sync the renderer runs on load, not a
user action.

## Search index (read model + indexer)

The `search` command/tool is a read over `SqliteSearchStore`
(`crates/oxplow-db/src/search_store.rs`) — a unified FTS5/BM25 index, not a
per-domain store. It is **fed**, not written by its callers: the `Indexer`
service (`crates/oxplow-app/src/indexer.rs`) is a single background task that
backfills at boot and then *subscribes to the EventBus* — `TasksChanged`,
`WorkNotesChanged`, `CommentsChanged`, `WikiPagesChanged`, and
`FileSnapshotCreated` / `FileSnapshotsBatchCreated` — and upserts/removes the
affected rows. This is the inverse of the usual "command writes store + emits
event" flow: the search index is a derived projection that *consumes* the same
events the UI does, so no command needs a special "also reindex" step. It's
spawned in `apps/desktop/src-tauri/src/main.rs` alongside the commit indexer.
Stream archive/delete calls `search_store.purge_stream` so a removed worktree's
file rows don't linger. Exposed identically on IPC (`search`) and MCP
(`search`) — see the parity manifest (`crates/oxplow-surface-parity`).

## Related

- [data-model.md](./data-model.md) — the actual schemas, including
  the `page_ref` graph and its column conventions; plus the global
  `recent-projects.json` / `instance.lock` state.
- [agent-model.md](./agent-model.md) — how the agent calls into MCP
  tools that wrap these stores, including `list_backlinks` /
  `list_outbound`.
