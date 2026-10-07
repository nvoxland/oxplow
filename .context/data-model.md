# Data model


What this doc covers: the SQLite tables, their store classes, and the
`sort_index` queue invariant for tasks. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Storage

All persistence lives in one SQLite file under `.oxplow/local.sqlite`, opened
through `Database::open` (`crates/oxplow-db/src/database.rs`). Every store is
a thin class wrapping that connection. Schema changes go through versioned
refinery migrations in `crates/oxplow-db/migrations/`, append-only; never
edit a prior version. **The history was squashed (tsk1080):** V1..V168
became one `V1__baseline.sql`, the schema they built, dumped from
`sqlite_master` with the rows they seeded; V2 (`change_file_stage`) is the
first after it, then V3 (`pending_run`, paced collector and change-analysis runs, and
`change.elapsed_ms` — metrics.md "Pacing"). The
`V<n>` names elsewhere in these docs are that history — which step brought
a table in — not files that exist. Existing databases were converted by a
one-off tool that checked their tables, indexes and triggers equal the
baseline's (materialized `m_v_*` tables aside) and rewrote
`refinery_schema_history` to the baseline row. With no other users, a
squash like that is the sanctioned way to drop history; editing a
migration in place is not.

> ⚠️ Migrations are **embedded at compile time** (`refinery::embed_migrations!`,
> a proc macro cargo knows nothing about). `crates/oxplow-db/build.rs` declares
> `rerun-if-changed=migrations` so a migration-only change (pure SQL, no `.rs`
> touched — V64 was the first) actually recompiles oxplow-db. Without it, cargo
> reuses the cached crate and every downstream binary silently ships WITHOUT the
> new migration — tests stay green, the app runs, the schema change just never
> happens. Don't remove that build script.

**Migrations never create views (P4.2).** A published view is a model file
(`crates/oxplow-db/models/`, see [semantic-layer.md](./semantic-layer.md)
"Models"). `migrate_and_compile` drops every model's view, runs the
migrations, then compiles the models — so a migration that alters or drops
a column never has to work around a view that reads it; the model that
reads it is fixed in the same change, and the compile at open says so if
it isn't. The build script embeds `models/` the same way
(`rerun-if-changed=models`).

**The model registry** (V105, V109): `model` (one row per published view
— owner `core` or an extension, `kind` `sql` or `entity`, version,
compiled SQL), `model_input` (its `ref`/`source` inputs), `model_contract`
(the columns each view promised at each version, and since V141 its
`key_json` — the declared `key:` columns, `'[]'` for the versions
recorded before keys; outlives the view, so a changed contract or key at
the same version is refused) and `model_test` (the last result of each
declared test). `model.materialize` is `on_change`, `every <duration>`
(V142) or `incremental <column>` (V143): SQLite can't widen a CHECK in
place, so both rebuilt `model`, keeping `model_input` and `model_test`
aside across the cascade. Readable as `v_model*`.

**Changed tables per commit** (P4.6, `crates/oxplow-db/src/changes.rs`):
every pooled connection's init installs SQLite's preupdate, commit and
rollback hooks; a commit's touched tables — and which of them it
rewrote (updated or deleted in, P8.B3) — are published after each
`Database::call` / `call_mut` / `transaction` / `read`
(`Database::subscribe_changes`), and `models_changed` maps them to models
through `model_input`. A write needs nothing to be seen — the hooks see
every table, WITHOUT ROWID and a bare `DELETE` included.

**DB dispatch is gated to the pool size (tsk131).** `Database::call`,
`call_mut`, and `transaction` each take a `tokio::sync::Semaphore` permit —
sized to the r2d2 pool (`max_size`) — *before* `spawn_blocking`, and hold it for
the duration of the DB work. Without the gate every caller spawned a blocking
thread regardless: the metric path fanned out to ~197 against 8 connections, so
~189 OS threads (≈2 MB of stack each, ~400 MB, matching observed RSS growth)
existed only to block inside `pool.get()`, and the profile counted **2,841
blocking threads created**. Throughput is unchanged — only `max_size` tasks
could ever hold a connection — the queue just moves to a cheap async wait.

It cannot deadlock: a permit is held only across the `spawn_blocking`, and those
closures are synchronous, so they can't await another gated call and permits
never nest. The in-memory test pool is `max_size(1)`, so the whole suite runs
through a **one-permit** gate — a standing stress of that property.

**Transactions.** One user-visible action that spans multiple writes
to the same DB must commit as ONE transaction. The pattern (see the
`transactional-boundaries-design` wiki page): store write ops are
split into sync `*_tx(conn, …)` cores (free functions in the store's
module, taking `&rusqlite::Connection`); the async store method is a
thin `db.call` wrapper over its core; multi-write actions compose
several cores inside one `Database::transaction(f)` closure
(`crates/oxplow-db/src/database.rs`) — which owns commit/rollback and
the bounded `SQLITE_BUSY` retry (safe because a rolled-back attempt
left no trace; that's why `f` is `Fn`, borrowing what it writes rather
than consuming it). It is the **one write path** (tsk978): no store opens
a transaction of its own on a raw connection — a test
(`every_store_write_runs_in_the_retried_transaction`) allows only
`database.rs` and the open-time model compile in `models.rs`. It begins **IMMEDIATE** (tsk503):
the write lock is taken at BEGIN and waited for under `busy_timeout` — a
BEGIN still Busy after that wait is retried like a Busy inside the
closure (tsk1005) — so
a read-then-write closure (the hook ingest, most `_tx` cores) can't fail
with `SQLITE_BUSY_SNAPSHOT` when another writer commits between its read
and its first write. A pure read therefore uses `Database::read` (a
DEFERRED snapshot that is always rolled back, no write lock), not
`transaction` — the effort, event-by-seq, worktree and turn-signal reads
and every `Read` command do (tsk513). **The event-log row is the one
write that belongs inside the closure**: a producer composes
`event_log_store::append_tx(tx, &envelope)` next to its state change
so the log and the state can never disagree (the outbox pattern; see
"event_log" below). Everything downstream of the log — the in-memory
`OxplowEvent` broadcast that wakes the UI, `page_ref` projections,
snapshot requests — runs AFTER commit, never inside the closure. Don't
convert existing single-op methods preemptively — extract a `_tx` core
the first time an op needs to join a transaction. Current users:
the task-store status cores (`set_status_tx`,
`update_with_status_tx`, `insert_logged_tx`), the effort cores
(`start_tx` / `finish_tx`), `record_take`, and every `Tx` command
handler (which runs in the bus's transaction).

**Lifecycle invariant.** At most one effort is open per thread (the
V5 unique index `idx_effort_open_per_thread`), and a task's status never
opens or closes one: `task_store::apply_status_tx` (the core of
`set_status_tx` / `update_with_status_tx` / `insert_logged_tx`, behind
`work_item.transition` / `update` / `create`), `soft_delete` and
`move_task` touch the task alone — `get_task_tx` sees live rows only, so
a deleted task takes no edits. Efforts open, close and link through the
`effort.*` commands, which the effort policy runs as it reacts to
`work_item.state_changed` (`.context/work-tracking.md`). Boot recovery
(`crates/oxplow-app/src/recovery.rs`) only closes turns the previous run
left open: an effort open across a restart is legal. Snapshot pins are
taken after commit by the effort-lifecycle consumer
on the event pump (`set_start_snapshot` / `set_end_snapshot`; see
"event_log" → async consumers) — an effort row is never gated on
snapshot success, and a crash between the two is resumed, not lost.

**Error typing.** Store failures surface as typed `DomainError`
variants, not stringified blobs: `map_sql_err`
(`crates/oxplow-db/src/database.rs`) classifies rusqlite errors —
constraint violations → `Constraint`, `SQLITE_BUSY`/`SQLITE_LOCKED` →
`Busy` (the only `is_retryable()` variant), everything else →
`Storage`. `Invalid` is reserved for caller-supplied validation
failures. The IPC layer maps these to `CONSTRAINT` / `BUSY` /
`STORAGE` codes (`crates/oxplow-rpc/src/error.rs`). Don't map SQL
errors to `Invalid` in new store code.

## Entity ids

Every externally-visible entity id is a SQLite **autoincrement INTEGER**
rendered with a fixed **3-letter type prefix** at the application boundary,
e.g. `str5` (stream), `thr21` (thread), `tsk38` (task), `eff7` (effort),
`not3` (note), `trn9` (agent-turn), `cmt2` (comment), `fup1` (follow-up).
The prefix lives only in Rust (`Display`/serde) and in the TypeScript
bindings (each id type is a `string`); **DB columns and the FKs that point
at them are plain `INTEGER`**. The single source of truth is
`oxplow_domain::EntityKind` (prefix ↔ label ↔ kind) in
`crates/oxplow-domain/src/ids.rs`; each id newtype (`StreamId`, `ThreadId`,
…) is an `i64` wrapper generated by one macro, with `AnyId { kind, value }`
as the type-erased form for polymorphic boundaries (the ref graph, the MCP
`expect_id_kind` validator). Rowid `0` is the `placeholder()` sentinel
(insert allocates a real id and returns it; AUTOINCREMENT never issues 0).
Two polymorphic TEXT columns stay TEXT and store the *prefixed string*
form: `page_ref.source_id`/`target_id` and `search_entry.ref_id`/`stream_id`
(`event_log.id` is a UUIDv7, also TEXT). Refs in prose/wiki use the bare
self-typed form, e.g. `[[tsk42]]`.

## Global state (outside `.oxplow/`)

Two pieces of state live **outside** any project's `.oxplow/` dir,
because they're global to the shell, which has no project backend of
its own (see [architecture.md](./architecture.md)). The app-config dir holding them
defaults to the platform location but is overridable with `OXPLOW_HOME`
(used verbatim; empty = unset) so a dev build can run alongside an
installed one — see [DEV.md](../DEV.md):

- **`recent-projects.json`** — the launcher's recent-projects list, in
  the app-config dir (`~/Library/Application Support/net.voxland.oxplow/`
  on macOS). Managed by `oxplow_config::RecentProjects`
  (`crates/oxplow-config/src/recent.rs`) — a JSON file, not a SQLite
  table, modeled on the LSP-installer manifest pattern. `record()`
  dedups by canonical path, bumps recency, caps at 20. The IPC layer
  resolves the path via the Tauri path resolver and manages it as
  `RecentProjectsState` on the shell.
- **`session.json`** — the set of project dirs open last, in the same
  app-config dir. Managed by `oxplow_config::SessionProjects`
  (`crates/oxplow-config/src/session.rs`); drives session restore on a
  bare launch. The shell **`replace`s** it with its own open project
  windows whenever one opens or closes — it knows the set rather than
  inferring it. Closing the last window, or a full quit
  (`ShellWindows::begin_quit` from `RunEvent::ExitRequested`), leaves
  the set alone so it's what comes back. Read-modify-writes still take a
  cross-process `fs2` lock (a dev build and an installed one can share a
  config dir) — taken on a `<name>.lock` sidecar, because the document
  itself is replaced by `rename`. **Both global documents are written
  atomically** (temp file + `fsync` + `rename`, `oxplow-config`'s
  `atomic.rs`): a truncate-then-write that died midway left an
  unparseable file, and both stores read an unparseable file as an empty
  document — so the failure mode was silent loss of the whole restore
  set or recents list (tsk253). The path is resolved by
  `oxplow_config::global_config_dir()` so `main.rs` can read it before
  any Tauri handle exists.
- **`.oxplow/instance.lock`** — a per-project advisory lock (fs2) held
  for the life of the project's `oxplow-daemon`, so a second daemon
  can't boot on the same `local.sqlite`. Helpers:
  `AppLayout::instance_lock_path` /
  `oxplow_app::{try_acquire_instance_lock, is_project_locked,
  wait_for_project_unlock}`. It lives inside `.oxplow/` but is not part
  of the SQLite schema.
- **`.oxplow/daemon.json`** — `{ base_url, pid }` published by a running
  daemon. The shell that spawned it learns the endpoint from its stdout;
  this file is for the *other* case — a daemon that outlived the shell
  that started it, which the next open finds and kills
  (`oxplow_app::daemon_supervisor::{write_daemon_info,
  kill_orphan_daemon}`).

## Tables and stores

### `streams` — `StreamStore` (`crates/oxplow-db/src/stream_store.rs`)

Top-level workspace context. Exactly one row per user-facing stream (a stream row in the Navigator).
Each stream owns:

- a `kind` column (migration v34) — `"primary" | "worktree"`:
  - **primary**: the repo itself. `worktree_path === projectDir`,
    `title === projectBase` (never rewritten), created exactly once at
    startup by `Services::boot()` via `StreamStore.findPrimary()`.
    Cannot be deleted (`StreamStore.deleteStream()` throws for `kind === "primary"`).
  - **worktree**: a real git worktree at
    `<parent_of_project>/<project_basename>-<slug>/` (sibling of the
    main repo) created by `createStream()` via `ensureWorktree()`.
    Pre-migration rows that point at the legacy
    `<project>/.oxplow/worktrees/<slug>/` location keep their stored
    `worktree_path`. Title defaults
    to the branch name; the runtime rewrites the title when the branch
    switches only if the old title matched the old branch (preserves
    user renames).
- a `branch` / `branch_ref` pair that is **not** pinned — any stream can
  switch branches. Updated by `StreamStore.setStreamBranch(streamId,
  branch, branchRef)`, which emits `stream.changed` (kind:
  `"branch-changed"`). The runtime drives it from two sites:
  `Services.checkoutStreamBranch(streamId, branch)` (user-triggered
  via the `BranchPicker` in the `TitleBar`) and
  `maybeSyncStreamBranch(streamId)` (fired by every `git-refs.changed`
  event so external `git checkout` in a worktree is picked up live).
  Git-level errors (dirty tree, missing branch, branch already checked
  out in another worktree) bubble through unchanged so the UI inline
  error shows git's own message.
- a worktree path (projectDir for primary; sibling
  `<parent>/<project_basename>-<slug>/` for worktree kind — the
  `<slug>` is fixed at creation and does not rename on branch switch)
- two agent pane targets (`working` and `talking`)
- per-pane / per-thread agent resume session ids (so reconnecting picks up
  history for the assigned agent)
- a `runtime_state.current_stream_id` pointer (singleton row, id=1)
- a `sort_index` column (migration v14) — streams are listed ordered by
  `sort_index, rowid`. `reorderStreams(orderedStreamIds)` reassigns
  sequential indexes and emits a `stream.changed` event (kind:
  "reordered"). **No UI reaches it today** (tsk272): the drag-to-reorder
  lived in `StreamRail`, which the `Navigator` superseded without
  carrying the gesture over; `App.handleReorderStreams` is still defined
  but is passed to nothing. The store, IPC, and handler all work — only
  the gesture is missing. Any replacement must keep **primary-first**
  regardless of sort_index (the primary stream can't be dragged and
  nothing may drop before it).
- a `custom_prompt` column (migration V6, nullable TEXT) — per-stream
  standing instructions appended to the agent's system prompt after the
  global `agentPromptAppend` section. Set via the `stream.set_prompt`
  command (`apps/desktop/src/api.ts`'s `setStreamPrompt`); the stream
  list re-reads on `ModelsChanged` naming `v_stream`.
  V6 also dropped a legacy `summary` column that was carried over from
  the TS schema and never displayed; the prompt feature briefly
  piggybacked on that slot before getting its own column.

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
by the rail's "Remove…" action. `StreamStore::list` filters
`archived_at IS NULL` so archived streams disappear from the rail
without dropping rows that history (efforts, snapshots, page_visit)
references. Pre-migration rows stay NULL and remain visible. Primary
streams cannot be archived.

### `threads` — `BatchStore` (`crates/oxplow-db/src/thread_store.rs`)

Units of work *within* a stream. Statuses: `active` (writer — may mutate
the worktree) and `queued` (read-only, agents can run but writes are
denied — see [agent-model.md](./agent-model.md)'s write-guard section).
Exactly one thread per stream is `active`; the rest are `queued`. A
newly-seeded stream ships with one thread titled `Thread`, running the
project's default agent — `oxplow_config::default_thread_agent`, the
same rule `thread.create` uses when no agent is named: the first enabled
agent, and for `acp` the project's first `acpAgents:` entry, else the
first preset (tsk970). `StreamService` reads it through the source
`Services` gives it, so it's the config as it is when the thread is
made.
The rolling `summary` field + `record_batch_summary` MCP tool were
removed in v13 — use the task log as the source of truth instead.

`closed_at` (migration v44, nullable TEXT) — closing a thread is
orthogonal to its status. A closed thread keeps its `queued` status but
sits hidden from the rail; it surfaces only on the **Closed Threads**
page (`apps/desktop/src/pages/ClosedThreadsPage.tsx`), which lists each closed
thread with its tasks (read-only) and a Reopen action. Closing is
allowed only for non-writer threads with no tasks in `ready` /
`blocked` / `in_progress` (`ThreadStore.close()`); promote another
thread or finish/move open items first. `ThreadStore.reopen()` clears
`closed_at` and the thread returns to the rail as a queued read-only
tab. `ThreadStore.list()` already filters closed threads out, so every
existing surface (rail, work panel, dispatch) treats them as gone
without code changes; `listClosed(streamId)` is the dedicated reader
for the Closed Threads page. Migration v44 also retires the legacy
`completed` status — any pre-existing `completed` rows are remapped to
`queued` with `closed_at = updated_at`.

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
when the parent stream's "Remove…" action archives every thread under
it (separate from `closed_at`, which is the user-visible "I'm done
with this thread" gesture). `ThreadStore::list_for_stream` filters
`archived_at IS NULL`, so archived threads vanish from every consumer
in lockstep with the stream that owns them.

`custom_prompt` (migration v18, nullable TEXT) — per-thread standing
instructions appended to the agent's system prompt after the stream-level
`custom_prompt`. Set via `setBatchPrompt(threadId, prompt)` on `BatchStore`;
IPC-exposed as `setBatchPrompt(streamId, threadId, prompt)`. Emits a
`thread.changed` event (kind: "prompt-changed") so the UI refreshes thread
state.

`agent` (migration V30, non-null TEXT) — the agent implementation assigned
to the thread at creation time. Values are `claude`, `codex`, or
`opencode` (V32 widened the CHECK; because migrations run with
`foreign_keys=ON`, V32 swaps the column — ADD/copy/DROP/RENAME —
instead of rebuilding the table, since `DROP TABLE threads` would
cascade-delete child rows). Project
config (`.oxplow/project.yaml` `agents: [...]`) controls which values can be selected
for new threads and the first configured agent is the default. Existing
threads migrated at V30 default to `claude`; the assignment is immutable in
v1 so resume/session history stays unambiguous.

`agent = 'acp'` (V90, tsk335) is an agent spoken to over the Agent Client
Protocol.

- **Which agent:** `acp_agent` (nullable TEXT, V90) names it, a preset
  (`claude`, `gemini`, `codex`) or a project `acpAgents` entry.
  `ThreadService::create_with_acp` enforces that an ACP thread names one
  and only an ACP thread does.
- **Migration:** V90 is the same column swap as V32, and it drops and
  recreates `v_thread` around it (a view reading the column blocks
  `DROP COLUMN`). The recreated `v_thread` adds `acp_agent`.
- **Test:** `v90_keeps_thread_children_and_accepts_acp` migrates a real DB
  to V89, adds children, then finishes; the children must survive.
- **Session state:** `threads.resume_session_id` holds the ACP session id
  (`session/load` on the next open). The conversation itself has **no
  table**: it lives in memory (`acp::transcript::Transcript`) and a load
  replay rebuilds it. What an ACP turn records lands in the same tables as
  a hooked turn (hook events, agent turns, tool calls, effort files,
  token usage).

**Removed in v42:** the `auto_commit` column (added in v15) and the
`commit_point` / `wait_point` tables (added in v6/v7). Commits are now
user-driven only — the harness has no `git commit` path, no queueable
commit/wait markers, and nothing that steers the agent to commit. Consumers
running an older DB get the columns/tables dropped on first launch with
the new binary; existing rows are not migrated forward (no surface
reads them).

### `task` — `SqliteTaskStore` (`crates/oxplow-db/src/task_store.rs`)

The actual TODO list. Singular table name, `id INTEGER PRIMARY KEY
AUTOINCREMENT` — stored as a plain integer, surfaced as `tsk<int>` (see
[Entity ids](#entity-ids)). The
`kind` column was dropped: there is no `epic`/`subtask`/`bug`/`note`
discriminator any more. An **epic is any task that has children** —
i.e. any row that's a `parent_id` target. The UI's data layer
(`workItems.bucketThreadWork`) computes this on read and the renderer
reads `ThreadWorkState.epics`; there's no flag on the row itself.

Statuses: `ready`, `in_progress`, `blocked`, `done`, `canceled`,
`archived`. `archived` is a terminal state that hides the item from
the default Work panel view — archived rows fold into the Done
section's bucketing but aren't rendered unless the user flips the "Show
archived (N)" toggle in the Done section header. The same header carries
an "Archive all" action that bulk-archives every visible Done/Canceled
row. The orchestrator's `read_task_options` blocker check treats
archived the same as done/canceled. `parent_id` chains items under
epics. The `description` (markdown) is the single prose field, structured
however the author sees fit — the model is not prompted toward any
template, and is meant to be human-readable, wiki-formatted for
readability. Task links express dependencies (`blocks`, `discovered_from`,
`relates_to`, …) via the `task_link` join table.

`thread_id` is nullable — items with `thread_id IS NULL` belong to the
**backlog** (a global, stream-less queue). The store API uses the constant
`BACKLOG_SCOPE` as a sentinel string in event payloads so listeners can
distinguish backlog changes from in-thread changes.

`author` (migration v26, nullable TEXT) — semantic origin of the row,
distinct from `created_by` (which just classifies the SQL writer as
`user`/`agent`/`system`). Values: `'user'` (explicit user-initiated
create), `'agent'` (a `work_item.create` run by an agent-driven
actor), or `NULL` (legacy rows). Pre-v29 DBs
also held `'agent-auto'` rows synthesized by the removed auto-file
listener; migration v29 cancels any such still-in_progress rows, and
the read path maps the legacy string to `null` so older terminal rows
continue to load under the narrowed enum.

The `description` column is the task's single canonical prose body.
(An earlier developer/executive/terse audience-variant feature added a
`description_variants` column in V27; it was removed in V29 along with
`effort.summary_variants` and `comment.section_anchor` — see the
V29 migration. Tasks/efforts/wiki pages now carry one body each.)

`note_count` is a computed column added to every `Task` returned by the
store (via COUNT subquery over `task_note`). It drives the note badge on
list rows and is always 0 when no notes exist.

`category` and `tags` were removed in V19 (`drop_task_category_tags`).
Both were settable via the rail UI and MCP create/update params but
nothing read `tags`, only `BacklogDrawer` showed `category`, and
nothing populated either automatically — they were vestigial. The
Backlog page never actually grouped or filtered by them despite the
old doc claim.

`completed_at` is set on entering `done` and kept when a done task is
archived (archiving is tidying, not undoing); moving anywhere else clears
it (`Task::set_status`).

### `work_item` — every provider's work items (migration `V115__work_item.sql`, P5.C1)

One row per work item, keyed by its ref (`work_item:<provider>:<id>`):
`provider`, `title`, `body`, canonical `state` (`todo`, `in_progress`,
`blocked`, `done`, `canceled`), the provider's own `native_state`, its
other fields as `native` JSON, `parent_ref`, timestamps and
`deleted_at`; since V17 the interface's own `thread_id` (the list it's
on, NULL = backlog), `rank` (order on the list) and `closed_at` (when it
first reached done or canceled; reopened, NULL). Published as
`v_work_item`: live rows **of the active work list only** (a join on the
active `work_items` row of `capability_provider`, so with none it's
empty). `v_task` stays oxplow's native model — read only by oxplow's
implementation (and `oxplow-dev`).

**`work_item_link`** `(from_ref, to_ref, link_type, created_at)` and
**`work_item_comment`** `(id, ref, body, author, created_at)` (V17) are
the interface's links and comments, published as `v_work_item_link` /
`v_work_item_comment` over the active list's items. oxplow's follow
`task_link` / `task_note` (a task's notes) by triggers, however those are
written (V17 backfilled them; a comment's id is `task_note:<id>`).
`capability_provider.fields_json` (V18) is an implementation's declared
fields. Capability rows are published when services are built
(`CapabilityRegistry::publish_now`), so the first read sees the active
list.

The oxplow provider's rows (`work_item:oxplow:tsk<n>`) are restated from
the `task` row by `task_store::project_work_item_tx`, which every task
write calls in its own transaction (insert, field update, status,
soft delete) — the two never disagree. Mapping: `ready` → `todo`;
`archived` → `done` when `completed_at` is set, else `canceled`; the rest
by name. The interface columns: `thread_id` is the task's thread, `rank`
its `sort_index`, `closed_at` its `completed_at` (else when it closed). (Archiving keeps `completed_at`; before V115 it cleared it, so
V122 restored it — and `done` — for every task whose last archive the
event log shows came `from: done`. Tasks archived before the event log
existed have no record and stay `canceled`.) `native` carries priority, thread, sort index, author and
`completed_at`. A task deleted by a cascade (its thread or stream
deleted outright) never passes through the store, so a trigger
(`work_item_follows_task_delete`) deletes its row. An external
provider's rows arrive by projection from its events (P5.C2); its
`thread_id` is the thread that filed it (first record), `closed_at` when
a record first closed it.

### `symbol` + `symbol_capture` — code symbols (migration `V117__symbol.sql`, P5.C6)

The symbols the running language servers report for each stream's
files, restated per changed file at each snapshot by the
`symbols.collect` pump consumer, and one `symbol_capture` row per
snapshot saying what the collection covered. Read as `v_symbol` /
`v_symbol_capture`; details in [lsp.md](./lsp.md) and
[semantic-layer.md](./semantic-layer.md).

### `ai_result` — recorded AI computations (`crates/oxplow-db/src/ai_result_store.rs`, migration `V118__ai_result.sql`, P5.E1)

A `classify` / `score` / `summarize` / `extract` result, `UNIQUE
(input_hash, provider, model, prompt_version)` (the provider since V121,
which rebuilt the table taking each row's provider from its `ai_call`),
with its `op`, `role`, first
`caller`, `output_json`, tokens and the computing `ai_call_id`; a
concurrent duplicate keeps the first (`ON CONFLICT DO NOTHING`). The
same migration adds `ai_call.input_hash`. Read as `v_ai_result`; see
[ai-providers.md](./ai-providers.md) "Recorded computations".

### `work_note` — thread-scoped notes only (`crates/oxplow-db/src/work_satellite.rs`)

Structured per-thread notes. Each row has `id`, nullable
`task_id` (kept for legacy rows), nullable `thread_id`, `body`,
`author` (free-form string, e.g. "agent", "user",
"explore-subagent"), and `created_at`. A CHECK still enforces that
**exactly one** of `task_id` / `thread_id` is non-NULL.

**Item-scoped writes were retired** — `effort.summary` is
the canonical record of what shipped on a task, so a parallel
per-item note table for the same purpose was duplicative. The
`add_work_note` MCP tool, the `add_work_note` / `list_work_notes`
IPC commands, and the task modal's "Notes" timeline section
were removed alongside this. The (since-deleted) MCP `complete_task` previously still
shadow-wrote the summary into `task_note` to get its body
projected into `page_ref`; that orphan write was removed once
the effort store learned to project `effort.summary`
directly (see the `summary_*` ref_types in the `page_ref` section
below). Pre-existing item-scoped rows stay in the table but no
surface reads or writes them.

Thread-scoped rows (`thread_id` set, `task_id` NULL) are the per-thread
capture pad, written by `knowledge.add_note` / `knowledge.update_note`
(P8.A6). An orchestrator handing a question to an Explore subagent
allocates an empty note first and the subagent fills in its body with
`knowledge.update_note`. The orchestrator reads them
back via `oxplow__list_thread_notes` / `listThreadNotes(threadId)` —
reverse-chronological, capped at 100.

### `effort` — `EffortStore` (`crates/oxplow-db/src/effort_store.rs`)

**V5 (inferred work tracking, [work-tracking.md](./work-tracking.md)).**
An effort is a span of one thread's work, and oxplow is moving to open
and close it itself. V5 made `work_item` nullable (NULL = unlinked; the
`''` the old default stored became NULL), allowed **at most one open
effort per thread** (`idx_effort_open_per_thread`; the migration closed
every older open one per thread as `system`) and dropped the
one-open-per-work-item index. It added `title` (an override; `v_effort.title`
is it, else the item's title, else the first line of the thread's first
prompt in the span) and `closed_by` (`commit`, `switch`, `person`,
`agent`, `system`). Done by column — add, copy, drop, rename — never by
rebuilding the table, which would cascade away the child rows. In Rust
`Effort.work_item` is `Option<String>`; `start_tx(conn, ev,
work_item: Option<&str>, …)` and `finish_tx(conn, ev, id, &EffortEnd)`;
the events are `effort.opened@2` / `effort.closed@2` (with `closed_by`) /
`effort.finished@2`, each v1 upcast as is, plus `effort.linked@1` and
`effort.retitled@1`. Opening an effort on a thread that has one open
closes that one first (`switch`).

**Adoption and as-of closes.** `start_tx(conn, ev, &EffortStart)` may
start an effort in the past (`adopt_since`, clamped to the thread's
previous effort's end): it then stamps itself onto the thread's
un-efforted rows since then in the derived tables that carry an effort
(`EFFORT_STAMPED`: `agent_tool_call`, `agent_token_usage`,
`metric_capture`, `agent_nudge`, `claim`, `decision`, `thread_answer`).
`finish_tx` at a past point releases the rows after it. The event log is
not restamped: an event keeps the anchors it was written with.
`effort_at_tx(thread, at)` names the effort whose span covers a moment.
Writers take the thread's open effort (`open_for_thread_tx`,
`find_open_for_thread`); the old "single open effort", list-of-open and
target-overlap scoring went with the one-open-per-thread rule.
What follows describes the task-driven lifecycle still in place until
the effort policy lands.

An **effort** is one bracketed span of work on a **work item**, today
one `in_progress → done` (or blocked/canceled) cycle of a task. V100
(P2.5a, tsk427) renamed `task_effort` → `effort` and `task_effort_file`
→ `effort_file` in place (RENAME / ADD COLUMN / DROP COLUMN, never DROP
TABLE, so every child row survived; see the migration's header) and
replaced the `task_id` FK with `work_item TEXT NOT NULL`, a canonical
ref (`work_item:oxplow:tsk42`, or another provider's
`work_item:issues:ENG-12`; built with `refs::build::work_item_ref`).
`v_effort` / `v_effort_file` carry the ref only (no oxplow task id: a
reader joins `v_work_item` on it, so it sees the active list's item and
nothing with none). An effort goes with its thread (`thread_id … ON
DELETE CASCADE`), not its work item: an effort another stream's thread
linked to a deleted item survives as history — readers LEFT JOIN
`v_work_item`. Claims and decisions name the item they were made on the
same way (`claim.work_item`, `decision.work_item`, V22, which rebuilt
both tables from their old `task_id` foreign keys); so do metric facts'
spine dimension `oxplow.work_item`.

In Rust (P2.5b, tsk428) the row is `Effort { work_item, … }` with
`task_id() -> Option<TaskId>`; `EffortStore` (was `TaskEffortStore`)
is keyed by the ref — `start(work_item, …)` refuses anything that isn't
a `work_item` ref (`refs::build::validate_work_item_ref`),
`find_open_for_work_item`, `most_recent_for_work_item`,
`list_for_work_item`, `work_item_for_effort`. The effort's `page_ref`
slice is projected from the work item's provider-scoped id, so another
provider's item gets edges too. The UI mirrors the helpers in
`apps/desktop/src/workItemRef.ts` (`workItemRef`, `taskIdOfWorkItemRef`,
`workItemLabel`); an effort on another provider's item shows its label
and has no task page. Columns: `work_item`,
`thread_id`, `started_at`, `ended_at`,
`start_snapshot_id`, `end_snapshot_id`, `summary` (free-form text
an optional `effort.report` wrote describing what shipped; `v_effort`
(v3) reads it, else the final message of the effort's last turn),
`impacts_json` (V12 — nullable TEXT holding a JSON array of declared
`TaskImpact` rows of the form `{kind, id, action?}`; the LLM uses this
to call out wiki pages it created, tasks it completed, commits it
referenced, etc. Each row projects into `page_ref` under
`ref_type=impact` with the action carried in `source_extra`).
Opened and closed by the `effort.*` commands (the effort policy runs
them as items start and finish; `.context/work-tracking.md`):

- an open pins a start snapshot, linked to `start_snapshot_id`;
- a close pins an end snapshot, linked to `end_snapshot_id`.
  Capture de-dupes by content hash — an unchanged tree reuses the
  latest existing snapshot id rather than writing a near-identical row
  — so `end_snapshot_id` is set whenever the stream holds any snapshot
  (on a no-op close it falls back to the effort's `start_snapshot_id`).
  It is null only while the effort is open: `end_snapshot_id` null ⇔
  effort in progress. There is no time-based minimum gap.

`summary` is the effort's single canonical prose body, written by
`effort.report` on the thread's open (else latest) effort; nothing writes
it otherwise — `v_effort.summary` falls back to the last turn's
`agent_turn.answer` at read. Closing a thread (`thread.close`) or
archiving its stream (`stream.archive`, end snapshot taken first) closes
its open effort, `closed_by` `system`.

Re-opening a task (done → in_progress) on a thread gives it a second effort. At most one effort is open per thread.

`effort_file` records an effort's files. Columns: `effort_id`, `path`,
`change_kind`, `local_snapshot_id`, `closest_vcs_rev`, `vcs_rev_exact`,
`source` (V7: `claimed` | `observed`), primary key `(effort_id, path)`;
read as `v_effort_file`. Two writers, both as the work happens: the
`effort.claim` reactor claims each structured edit
(Edit/Write/MultiEdit/NotebookEdit) for the effort it happened in
(`record_file`, `INSERT OR REPLACE`, `source = 'claimed'` — a claim
replaces an observation); the `effort.observe` consumer records every
other path a thread's turn changed as `observed` on the effort holding
the turn (`observe_files_tx`: skipped when another thread's overlapping
effort claimed it, never overwriting a row). A file two threads changed
at once and neither claimed is observed by both. Nothing is declared by
the agent and nothing is reconciled at close.
See agent-model.md's "Per-effort write log" for the flow. Consumed by `get_effort_files`
(`crates/oxplow-tauri-ipc/src/commands/effort.rs`) over the
`EffortStore` and `SnapshotStore`: when ≥2 efforts share an end
snapshot AND this effort has ≥1 row here, the pair-diff is filtered
to those paths; 0 rows → fall back to raw pair-diff ("assume all");
1 effort → raw pair-diff.

Read API: `listEffortsForTask(itemId)`, `listOpenEfforts()`,
`listEffortsForSnapshot(snapshotId)`,
`listEffortsForPath(path)` (closed
efforts that touched `path` via `effort_file`, joined to the
owning task's title/status, newest-first by `ended_at` — drives
the local-blame overlay described in `.context/editor-and-monaco.md`).
The task page reads a task's efforts and each one's changed files in one
read over the models — `v_effort` left-joined to `v_effort_file`
(`workItems.readTaskEfforts` → `effortDetailsFromResult`), not an RPC
per effort.

**Commit↔item attribution is intentionally NOT tracked.** A
`task_commit` junction (migration v27) was never written or read and
was dropped in V100, with the equally dead `task_effort_turn`. Users commit outside oxplow all the time (IDE buttons,
CLI, CI rebases, merges, squashes) and oxplow has no authoritative hook
there. A blame-style feature built on that data would lie more often
than it'd be useful. If a future feature wants "show me commits for
this item," the answer is to scope `git log` by the files
in `effort_file`.

A **run** (a test, coverage or analysis capture) is the effort its
causing tool call was in: `metric_capture.effort_id`, stamped at ingest
from the tool event's effort anchor (else the thread's open effort) and
restamped when an effort adopts the turn. `v_test_run.effort_id` (v2)
reads it directly. `metric_capture.effort_id` is SET NULL on effort GC, so
the capture rows outlive their effort. See `.context/metrics.md` for how
effort reads use it.

### `snapshot` + `file_snapshot` — `SnapshotStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Time-ordered snapshots in two tables (the actual schema; an earlier
draft of this doc described a single `snapshot_entry` table with a
`version_hash`/`source` manifest — that pre-V13 shape is gone):

- **`snapshot`** (V13/V16/V42) — the grouping row, one per
  `request_snapshot()` call that had dirty files. Columns: `id,
  stream_id, created_at, revision, branch` (renamed from `git_commit`
  / `git_branch` in V112). `revision` is the VCS revision the workspace
  was clean against (`git:<sha>`, a `Revision`; else NULL); it's
  re-stamped in place when the head moves but the tree didn't change.
  `branch` (V42) is the branch the workspace was on at capture (set
  unconditionally, dirty or clean, via `Vcs::head`; NULL for pre-V42
  rows / detached head / not under version control) — it lets callers tell snapshots captured on different
  branches of the *same* stream's worktree apart (the diff page's
  snapshot picker filters by it).
- **`file_snapshot`** — the per-path rows: `id, stream_id, path,
  blob_hash, size_bytes, captured_at, storage, snapshot_id, mtime_ms,
  content_hash`. Each points back at its `snapshot_id`. `storage` (V37)
  is the explicit class telling you where the bytes live and how to read
  `blob_hash` back — see "Storage classes" below.
- **`snapshot.tree_hash`** (V96) — whole-tree identity: the xxh3-128 of
  the sorted manifest `path \0 identity \n` of the reconstructed tree
  (`oxplow_db::snapshot_tree::manifest_hash`), set by every capture. Equal
  trees hash equal; an un-hashed git entry makes it conservative (an
  extra snapshot, never a missed change). NULL before V96.

**Content identity vs storage address (V96, tsk423).** `blob_hash` is
the **address** (where the bytes live); `content_hash` is the
**identity** (what the bytes are): the xxh3-128 of the bytes, for every
class that has bytes. They coincide for `oxplow` rows (capture fills
`content_hash` from `blob_hash`). A `git` row's address is a git blob
OID — a different hash space — so its `content_hash` is filled
**lazily**: `SqliteSnapshotStore::resolve_for_compare` hashes exactly the
git entries that sit opposite a different identity in a comparison (via
the `ContentHasher` the app wires over the VCS object store —
`SnapshotContent::object_content_hash` — in `Services::new`)
and persists the result by OID. A clean baseline is never re-read.
Everything that compares content goes through typed trees
(`tree_at` → `SnapshotTree` of `TreeEntry { storage, address,
content_hash, … }`, `TreeEntry::identity()`), never through an address:
`diff_snapshots`, `Trees` (`.context/vcs.md`; it knows each cell's class
instead of guessing from the string's length), change analysis, diff coverage
and wiki drift. `stats_for_snapshot` / `list_changes_for_snapshot`
classify by the same identity, so a row whose bytes equal its
predecessor's is not a change.

**`stream_id` is NOT NULL** (V16). Every captured row belongs to a
specific stream's worktree — different streams have independent
histories, and `listSnapshotsForStream` queries `WHERE stream_id =
?`.

**One `SnapshotCaptureService` per stream.** Snapshot capture is
*per-worktree*: `SnapshotCaptureRegistry`
(`crates/oxplow-app/src/snapshot_capture_registry.rs`) owns one
service per active stream, each watching its own
`worktree_path`. `Services::boot` enumerates every active stream
and registers a service for each; the stream lifecycle
commands (`stream.create_worktree`, `stream.adopt_worktree`,
`stream.archive`) register/unregister at
runtime. Callers resolve the right service via
`snapshot_captures.get(&stream_id)` (when the stream is known)
or `snapshot_captures.primary()` (for project-shared surfaces
like the wiki page watcher and freshness checks). `TaskService`
resolves via task → thread → stream, so lifecycle snapshots
capture against the task's actual worktree — a task on a
worktree stream never bleeds into the primary's snapshot
history.

`unregister` calls `SnapshotCaptureService::shutdown()`, which
fires a `tokio::sync::Notify` the `spawn_watcher` task selects on
alongside `rx.recv()`. This is required because the watcher task
holds its own clone of the service and its `FsWatcher` is a
task-local — dropping the registry's `Arc` alone never wakes the
task, so without the signal an archived stream's watcher would
linger until process exit.

**Turns are bracketed by snapshots (V98 P2.3 tsk425, V99 tsk438).**
`agent_turn.start_snapshot_id` is the stream's current snapshot when the
turn opened (set in the open transaction); `snapshot_id` is the snapshot
the worktree was at when it ended — set in the `turn_end` take's
transaction (`record_take` stamps it whenever the take carries a
`turn_id`). The turn's changes are start → end, like an effort's. `agent_turn.task_id` is gone (nothing ever set
it). `agent_turn` open/close log `agent.turn.started@1` /
`agent.turn.ended@1` in the same transaction as the row change;
`event_log` has a `turn_id` index. `SqliteAgentTurnStore::
stream_has_open_turn` answers "is an agent mid-turn on this worktree?".

**Effort↔snapshot linkage lives on the effort row.** There is no
`effort_id` or `source` column on `snapshot` / `file_snapshot`
themselves — the bracket is recorded by `effort.start_snapshot_id`
and `effort.end_snapshot_id`, each pointing at a `snapshot.id`.
A closed effort's `end_snapshot_id` is non-null whenever the effort has
a baseline — capture de-dupes an unchanged tree to the latest existing
snapshot id rather than writing a near-identical row, and on a no-op
close the effort-lifecycle consumer (`TaskService::on_effort_closed`) falls back to the effort's
`start_snapshot_id`. `end_snapshot_id` is null only while the effort is
open (`end_snapshot_id` null ⇔ effort in progress); there is no
time-based gap.

**Change detection.** `latest_stat_per_path(stream)` is per stream
(another worktree's rows are a different history). The startup sweep
short-circuits on `(size_bytes, mtime_ms)`: a file whose stat matches its
latest row is presumed unchanged and isn't re-read. When the stat moved
and the file is clean vs HEAD, an OID equal to the prior address is
unchanged; a prior row in the other space (an xxh3) is compared by
hashing the file, so committing a file already captured dirty records
nothing. Incremental capture skips a path whose bytes equal its latest row's
content — by `content_hash`, or for a git-backed row not hashed yet by
the bytes' git blob OID against its address (tsk439) — so a touch or a
formatter no-op is not a change, an oversize file whose
size and mtime are unchanged, and a tombstone for a path already
deleted. The sweep's reverse-deletion pass tombstones oversize files too
(it used to key on `blob_hash`, which oversize rows lack).

**Takes and the operation log (`snapshot_op`, V97, P2.2 tsk424).** A
**take** (`SnapshotCaptureService::request_snapshot(TakeRequest)`) drains
the dirty set and hands the rows to `SqliteSnapshotStore::record_take`,
which writes — in **one transaction** — the new `snapshot` row (with
`branch`, `revision` when the tree is clean, and `tree_hash`), its
`file_snapshot` rows, one `snapshot_op` row and a `snapshot.taken@1`
event (anchors: stream, thread, turn, effort, snapshot). It is the only
production write path; `create_snapshot` / `capture` / `capture_batch` /
`set_snapshot_revision` / `set_snapshot_branch` remain as fixture
seeders for tests.

- `snapshot_op(seq, stream_id, snapshot_id, parent_snapshot_id, trigger,
  thread_id, turn_id, effort_id, at, elapsed_ms, budget_ms, over_budget,
  file_count)` (STRICT). `snapshot_id` is where the worktree is after
  the take; `parent_snapshot_id` where it was before (the stream's
  current snapshot = its latest op's `snapshot_id`). Readable as
  `v_snapshot_op`; listed newest first by `list_ops` (RPC + MCP
  `list_snapshot_ops { stream_id, limit? }`, P2.11).
- **A snapshot row carries its creating op** (P2.11): the stream listing
  (`list_snapshots_for_stream`) joins each snapshot's FIRST op for its
  `parent_snapshot_id`, `trigger` and `over_budget`. The diff view's
  "previous" for a single snapshot is that recorded parent
  (`previousSnapshotId` in `apps/desktop/src/diffViewModel.ts`), falling
  back to the next-smaller id only for a snapshot no op created; Local
  History shows the trigger and marks an over-budget take "slow".
- **Every take records an op**, including one that found nothing new:
  its op points at the unchanged snapshot (`parent = snapshot`,
  `file_count = 0`, `snapshot.taken.unchanged = true`) and no new
  snapshot row is written. Rows whose tree equals the parent's
  `tree_hash` are dropped in the same transaction (unchanged).
- `trigger` ∈ `turn_end, quiet, effort_start, effort_end, startup,
  manual, git_refs, head_moved, run_measured, legacy` (`oxplow_domain::snapshot::
  SnapshotTrigger`). V97 backfilled one `legacy` op per existing
  snapshot, parent = the previous snapshot of the same stream. `run_measured`
  (V161, `snapshot.taken@2`) is the take that pins a run's coverage and
  analysis to the code it measured (see [collection.md](./collection.md)).
- **HEAD moved on a clean tree** is its own op: `record_head_moved`
  re-stamps the snapshot the caller saw the clean tree at — refusing if a
  take has moved the stream on since (tsk440), and the git-refs path
  holds the take lock from its drain through the stamp — with the new
  `revision` (and flips every
  exact-pin file ref on it) with a `head_moved` op and `vcs.head.moved@1`,
  in one transaction. The git-refs listener runs a `git_refs` take first
  (draining anything dirty), then this.
- Takes are **serialized** per stream (`take_lock`) and each caller
  records its own op, so two threads ending turns on one worktree give
  two ops, usually on the same snapshot (§4.3: a snapshot taken for one
  thread's turn is also "at" the other's).
- `budget_ms` / `over_budget` record a caller's time budget and whether
  the take exceeded it (P2.3 gives turn-end takes one); over-budget is
  logged at warn and visible on the op and event, never silent.
- The in-memory bus gets `OxplowEvent::SnapshotTaken` **after commit**,
  only when something new was recorded (a new snapshot, or a HEAD
  re-stamp with 0 files) — the indexer, change analysis and the UI wake
  on it. (Snapshot fact collectors run from the logged `snapshot.taken`
  through the `collector.triggers` consumer instead, P7.B3.) It replaced `FileSnapshotCreated` (never
  emitted) and `FileSnapshotsBatchCreated`.

**Baseline is hidden from Local History.** The first snapshot per
stream has no predecessor, so there's nothing to diff against and
nothing meaningful to show. `listFileSnapshotsForStream` excludes it
(subsequent snapshots use it as their "previous"). The baseline still lives in the DB — only the
UI list skips it.

**Rows come with pre-joined labels.** `listSnapshotsForStream` joins
against `effort` to populate `label` + `label_kind` on each
`FileSnapshot` (task title + " — start"/" — end"); effort-end wins
over effort-start when the same snapshot is both. Unlinked snapshots
get `label: null` and the UI falls back to a generic label.

**Storage classes (`file_snapshot.storage`, V37).** The explicit
discriminator for where a row's bytes live; `blob_hash` is read
differently per class. Replaces the old implicit `(blob_hash NULL?,
oversize?)` 2-bit encoding (the dropped `oversize` boolean):

- `oxplow` — `blob_hash` is an **xxh3-128**; bytes live in oxplow's
  content-addressed blob store at `.oxplow/snapshots/objects/xx/yyyy…`
  (shared across streams for dedup). xxh3-128 (not SHA-256/blake3) was
  chosen because it's a local non-adversarial cache and ~30–50× faster
  on Apple silicon (`crates/oxplow-app/src/blob_store.rs`).
- `git` — `blob_hash` is an **object id in the VCS's object store**
  (a git blob OID); the bytes are *not* copied into the blob store —
  they're recovered on demand through `Vcs::object_store`. This is the
  **VCS-sourced baseline**: the startup sweep records clean tracked
  files (working-tree-identical to the head, per `Vcs::clean_baseline`)
  by their object id instead of reading + hashing + blobbing them, so a
  clean checkout of a large repo boots without re-blobbing the tree. The
  class keeps its persisted name `git` (P5.B3): renaming it would rebuild
  `file_snapshot`, the largest table, for a word — it names the VCS's
  object store, whichever VCS that is.
- `oversize` — `blob_hash` NULL; the file exceeded
  `snapshotMaxFileBytes`, so only `size_bytes` + `mtime_ms` are tracked.
- `deleted` — `blob_hash` NULL; a **tombstone** row marking the path
  gone as of this snapshot (the per-snapshot deleted-count and diff
  status depend on this row existing — deletions are NOT absent rows).

**The read seam.** Every consumer that wants a captured file's bytes
(workspace file view, snapshot restore, search indexer, MCP/diff
readers) goes through `oxplow_app::snapshot_content::SnapshotContent`
(`Services.snapshot_content`: the blob store plus the VCS object store),
which switches on `storage` so none of them can forget the VCS
fallback. `SnapshotStore::content_ref_for_path` returns a
`SnapshotContentRef { storage, hash }` for the same routing.

**Durability tradeoff of `git` rows.** A git-backed row is only as
recoverable as its blob's reachability. If history is rewritten
(rebase/squash that orphans the commit) and git GCs the object, those
bytes are gone — we deliberately never copied them. `read_blob` returns
`None` and the read seam surfaces `GitUnavailable` rather than panicking.
Uncommitted working-tree content always takes the `oxplow` path and is
never at this risk.

**Retention: content expires, records don't (tsk105).** Retention's job
is bounding the ON-DISK blob copies — the megabytes. The
`snapshot`/`file_snapshot` ROWS are **never deleted**: they are durable
records other subsystems replay — the per-path metric fold derives each
capture's restated set from them, and the ancestry anchors ride the
parent `snapshot` rows' git stamps — and they only weigh bytes. (The
pre-tsk105 row prune rotted the fold's replay inputs out from under
durable captures; deleting rows to save disk was aiming at the wrong
mass.) The daily cleanup (`SnapshotCaptureService::run_cleanup`, boot +
every 24h) GCs `.oxplow/snapshots/objects/` down to
`retained_blob_hashes(cutoff)`: every row inside the retention window
(default 7 days, `snapshotRetentionDays` in `.oxplow/project.yaml`)
plus each `(stream, path)`'s newest row at ANY age — so every
worktree's current tree stays viewable/rollbackable forever. Older
content reads degrade to "expired": `read_file_snapshot` → `None`
(`SnapshotFileError::Expired` in `oxplow_app::snapshot_files`), restores
refuse with an explicit message, never a half-restore.
Only `oxplow`-class rows hold blob-store hashes; `git` rows reference
the git odb, which this GC never touches. The blob store is shared
across all streams, so GC runs at the project level and dedupes
identical content across branches.

Cleanup runs at runtime startup and again once every 24 hours via
`runtime.runSnapshotCleanup` (wired in `initialize()`, cleared in
`dispose()`).

**Ignoring generated directories.** The fs-watcher and the snapshot
seeder share one filter: `shouldIgnoreWorkspaceWatchPath` in
`crates/oxplow-fs-watch/src/lib.rs`. It covers `.git/`, `.oxplow/logs/`,
`.oxplow/worktrees/`, and a hardcoded list of common build/cache dir
names (`node_modules`, `dist`, `build`, `target`, `.next`, `.turbo`,
`.cache`, `.venv`, `__pycache__`, …). Users can extend the list via
`generatedDirs: [...]` in `.oxplow/project.yaml` — names are single path
segments matched anywhere in the relative path, and apply to both
the workspace watcher and the snapshot store. No changes to
existing snapshots on toggle; newly ignored paths simply stop
appearing in future dirty sets.

Toggling the list at runtime (the UI's generated-paths control →
`set_generated` IPC) takes effect immediately, no restart: the
command rebuilds the `WorkspaceFilter` and calls
`SnapshotCaptureRegistry::set_workspace_filter`, which swaps the
filter on every live per-stream `SnapshotCaptureService` (each holds
it behind an `RwLock`) and on the copy used to build future ones.

### `symbol` + `symbol_capture` — `SqliteSymbolStore` (`crates/oxplow-db/src/symbol_store.rs`, migration `V117__symbol.sql`)

The symbol index (P5.C6): each stream's files' symbols as the running
language servers report them, restated per changed file by the symbol
collector at each snapshot (`ref` `symbol:<path>/<name>@snap:<id>`,
`snapshot_id` the snapshot it was read at, `name`, `kind`, `container`,
`language`, 1-based `line`/`col`/`end_line`/`end_col`), and one
`symbol_capture` row per snapshot handled (`files_collected`,
`files_over_budget`, `files_without_server`). Read as `v_symbol` /
`v_symbol_capture`; see [lsp.md](./lsp.md).

### `wiki_page` — `WikiPageStore` (`crates/oxplow-db/src/wiki_page_store.rs`)

The per-project wiki's pages. The file is the page: it lives at
`.oxplow/wiki/<slug>.md` (not committed — a personal knowledge base),
and the row — written with the file in one run — holds its text and
what's derived from it: `slug`, `title`,
`body_path`, `body_excerpt`, `body_size_bytes`, the parsed
`file_refs_json` / `dir_refs_json` (`[[dir:…]]`) /
`related_notes_json`, `created_at`, `updated_at`, and `body_hash` (V116:
the hash of the body it was written from), and `body` (V125, P6.E2:
the text itself, which `v_knowledge_body` publishes). The one writer is
`wiki_page_store::upsert_tx` / `delete_tx`, inside
`knowledge.write_page`'s transaction or the watcher's — see
[knowledge.md](./knowledge.md) for the write path, pins and hand-edit
convergence.

### `page_ref` — unified cross-page reference graph (`crates/oxplow-db/src/page_ref_store.rs`, migration `V11__page_ref.sql`)

One row per directed edge `(source) --ref_type--> (target)` across
every page kind. Replaces the in-memory `computeBacklinks` indexer
that used to live in the desktop frontend; both backlinks ("what
points at me?") and outbound ("what do I point at?") are SQL
queries against this table.

Columns: `source_kind, source_id, target_kind, target_id, ref_type,
source_extra, local_snapshot_id, closest_vcs_rev,
vcs_rev_exact` (PK on the first five). Indexes on
`(target_kind, target_id)` for backlinks, `(source_kind,
source_id)` for outbound, and `local_snapshot_id` for the
cascade-on-commit-attach update. `kind` is denormalised next to
`id` so kind-filtered queries don't need LIKE on a synthetic
combined column.

**File-ref versioning (V20).** Edges whose target is a file or
directory carry a snapshot pin so callers can tell how out-of-date
each reference is. `local_snapshot_id` always points at the
`snapshot.id` the edge was captured against; `closest_vcs_rev`
is the closest known git commit at capture time
(the id of `snapshot.revision` when the worktree was clean, else the
head, via `Vcs::head`);
`vcs_rev_exact = 1` when the local snapshot is byte-equal to
that commit. Non-file edges leave all three columns NULL / 0. When a
revision lands on a snapshot later
(e.g. the clean-restamp path in `SnapshotCaptureService`), the
write cascades: both `effort_file` and `page_ref` rows
pointing at that snapshot get their `closest_vcs_rev` set and
`vcs_rev_exact` flipped to 1. The capture-time resolver lives
in `oxplow_app::file_ref_version`; callers don't pass any of these
fields by hand — the effort claim / observe paths and the wiki sync
watcher fill them automatically.

**Wiki sync preserves unchanged pins.** `sync_from_disk_with_refs_versioned`
calls `SqlitePageRefStore::merge_source` (NOT `replace_source`).
Each edge is matched against the existing row by the PK; existing
edges keep their `local_snapshot_id` / `closest_vcs_rev` /
`vcs_rev_exact`. Only newly-added edges get the current
snapshot pin. Edges removed from the body are deleted. So editing
unrelated prose doesn't re-stamp every file ref's freshness — the
pin only advances when the body actively re-adds the ref OR when
the agent explicitly verifies it (next paragraph). The wiki sync
also strips `@<version>` literals (`@disk`, `@HEAD`, `@<sha>`,
etc.) from `[[…]]` and `(file:…@…)` / `(dir:…@…)` body forms
before parsing, and writes the normalised body back to disk —
version state lives in the row, not the prose.

**Verification edges under cited dirs survive a write.** One exception
to "edges not in the body are deleted": a `wiki_file_ref` edge whose
`target_id` is a file under a directory the body cites
(`[[dir:…]]`) is re-included (`path_under_any_dir`) so `merge_source`
preserves it and its pin. These are **verification edges**, made by
`knowledge.write_page`'s `verified_refs` when the agent verifies a fact
against a specific file it references only by directory. They self-clean: once
the covering `[[dir:…]]` ref leaves the body, the edge is no longer
re-included and gets pruned.

**`verified_refs` / `removed_refs`.** `knowledge.write_page` takes
the paths the agent re-read against the new body (`verified_refs`: re-
pinned to the current snapshot; a file under a cited directory is
materialized as an edge) and the paths it took out (`removed_refs`:
must be gone). Refs left in the body but in NEITHER list keep their
existing pin — that's how "this content relies on a stale source" stays
accurate ([knowledge.md](./knowledge.md)).
Skill prompt at `crates/oxplow-plugin/assets/oxplow-wiki-capture.SKILL.md`.

**User-facing Freshness view.** `v_knowledge_ref` (read by
`knowledge.ts`'s `readWikiFreshness`) joins `page_ref` with the latest
primary-stream `file_snapshot` per target path, a `stale` flag per ref.
`WikiFreshnessPage` renders
the table with per-ref + per-page "Mark verified" buttons, which
re-write the page with those refs in `verified_refs`
(`knowledge.write_page`). The wiki page chrome adds a `Freshness (N stale)` action
chip that routes to the page.

Every stored `(source_kind, source_id)` / `(target_kind, target_id)`
is a canonical ref's `(kind, id)` (see `.context/refs.md`, tsk404), so
`format!("{kind}:{id}")` is the ref and nothing needs a per-kind id
scheme:
- `wiki` — the slug
- `work_item` — `oxplow:tsk<n>` (`page_ref_projections::work_item_id`)
- `file` — the repo-relative path
- `dir` — the repo-relative path, no trailing slash
- `commit` — the full sha
- `finding` — the rowid as a string
- `task_note` — `not<n>`

V92 wiped the pre-canonical rows; the boot backfill regenerates them.

**Writers own slices by `ref_type`.** A single `(source_kind,
source_id)` can have rows from multiple owners — a task's
body-mention edges (from `task_store`), link edges (from the
link store), and touched-file edges (from the effort store) all
land under `(work_item, oxplow:tskN)` but with distinct `ref_type`s.
`SqlitePageRefStore::replace_source_for_ref_types` lets each
writer wipe + re-insert only the rows whose `ref_type` it owns,
so other owners' rows survive.

Writers (one per source kind / slice):

The vocabulary speaks of work items, whichever list they're on (V21
renamed the `task_…` spellings: a mention is `work_item_mention` /
`summary_work_item_mention`, a link `work_item_link:<type>`).

| Owner | Source | Slice (`ref_type`s) |
|---|---|---|
| `wiki_pages.rs` (`oxplow-app`) | `wiki:<slug>` | full source — uses `replace_source` |
| `task_store::upsert` | `work_item:oxplow:<id>` body slice | `work_item_mention`, `wikilink`, `wiki_file_ref`, `wiki_dir_ref`, `finding_mention`, `commit_mention` |
| `work_satellite::SqliteTaskLinkStore` create/delete | `work_item:oxplow:<id>` link slice | `work_item_link:blocks` / `relates_to` / … |
| `effort_store::record_file` + `effort_store::finish` + `set_impacts` | `work_item:oxplow:<id>` effort slice | `touched_file`, `summary_wikilink`, `summary_file_ref`, `summary_dir_ref`, `summary_work_item_mention`, `summary_finding_mention`, `summary_commit_mention`, `impact` |
| `analytics_stores::SqliteCodeQualityStore::append_finding` | `finding:<id>` | full source |
| `commit_indexer.rs` (`oxplow-app`) | `commit:<sha>` | full source — diff yields `touched_file`, message yields the same body-mention set |

The shared extractor `oxplow_domain::refs::extract(body) ->
ExtractedRefs` is the single parser used by every writer that
takes a free-text body, so wiki/task/commit-message ref
recognition stays in lock-step. Pure projections live in
`crates/oxplow-db/src/page_ref_projections.rs`.

Reader: `SqlitePageRefStore::list_backlinks(target_kind, target_id,
limit)` and `list_outbound(source_kind, source_id, limit)`. Both
are exposed as Tauri commands (`list_backlinks` / `list_outbound`,
which decorate each row with a best-effort `source_label` from
the source store) and as MCP tools of the same names.

Boot-time restate: `oxplow_app::page_ref_backfill::run(...)` re-
projects every existing task body, link and finding into the table,
idempotently — the graph's repair path after a migration that resets
`page_ref` (V92) or a writer's drift. It runs at boot only when the
schema version or the build changed since its last run
(`needs_repair`; recorded as `asset_state` row `page_ref_repair`):
between those the writers keep the graph current. The effort slice (touched files + summary refs
+ declared impacts) goes through the effort store's own
`project_effort_slice` for every work item with an effort
(`list_work_items`) — another provider's included (tsk452; a copy of the
projection in the backfill used to drop declared impacts on every boot). Wiki bodies and recent commits are covered by their
own initial-scan paths and don't need separate backfill.

The effort slice has three contributors that run independently —
`record_file` re-projects after each touched-file write, `finish`
re-projects after `summary` lands, and `set_impacts` re-projects
after the declared `TaskImpact` list is written. They share the
same `effort_ref_types()` set, so each call replaces the full
union and the other contributors' rows survive.

The `impact` ref_type uses a single string regardless of target
kind (the `target_kind` discriminates wiki/task/commit/finding/
file/directory). The action verb (`created`, `updated`, `deleted`,
`completed`, `reopened`, `referenced`, `resolved`, …) lives in
`source_extra` as `{"action": "..."}` so the UI can render
"created by task #42" without an extra query.

### `event_log` + `event_consumer_checkpoint` + `event_dead_letter` + `command_audit` — `SqliteEventLogStore` (`crates/oxplow-db/src/event_log_store.rs`, migration `V93__event_log.sql`)

The event log, built in P1.4 (tsk406). **State tables hold current truth; the log
records activity and state changes**, and it is written in the *same
transaction* as the change — the outbox pattern — so the two never
disagree. The envelope is `oxplow_domain::events::Envelope`
(`crates/oxplow-domain/src/events/mod.rs`): `id` (UUIDv7 text, sorts by
time), `type` (`namespace.name`, snake_case, validated on append), `v`
(schema version), `at`, `source` (`agent:thr3`, `human`, `lens:<id>`,
`system`, or `system:<component>` from `refs::build::system_source` —
`system:task_service`, `system:hook_ingest`, `system:snapshot_capture`),
`anchors` (nullable stream / thread / effort / turn / snapshot columns,
so per-anchor timelines are indexed range scans; an event that names a
thread also carries its stream; V153 adds a partial index on a
`work_item.recorded`'s item ref, for a provider read's "did this
change?" lookup — providers.md), `subject` (JSON array of canonical
refs built with `oxplow_domain::refs::build` and validated against the
kind registry on append, see [refs.md](./refs.md)), `payload` (JSON;
validated against `type@v`'s schema on append), `payload_hash` (reserved for
forgettable bodies stored by content hash), `cause`, `dedupe_key`
(UNIQUE — the emitter derives it from the occurrence, so an at-least-once
producer's second `append_tx` fails with `Constraint` and writes nothing;
`append_unique_tx` instead inserts with `ON CONFLICT (dedupe_key) DO
NOTHING` and reports whether it landed, so a concurrent twin is skipped by
the insert itself rather than a check before it).
`seq` (AUTOINCREMENT) is the delivery order; `id` is the public identity.

These are the schema's first **STRICT** tables; every later spine table
is STRICT too.

**Schemas (P1.5, tsk407).** `oxplow_domain::events::schema` has one
Rust type per `type@v` (`trait EventType { TYPE, V, Payload: JsonSchema,
upcast }`) and `EventSchemaRegistry`, which holds every type the log
accepts. It sits with the ref kinds in the running `Vocabulary { events,
kinds }`, held in the swappable `VocabularyHandle` `Services.vocabulary`
(P8.D1; see refs.md) — `Vocabulary::core()` is every core type; the golden files under `schemas/events/` are the
authoritative list (the `core_registry_knows_every_core_type_and_version`
test pins it). An extension's types join it as data, not Rust types:
`register_declared(extension, DeclaredEventType { event_type, v, schema,
summary, upcast })` (P8.D2) refuses a core namespace (`CORE_NAMESPACES`,
§5.3), any namespace but the extension's own (`plugin_namespace`: its
name with `-` read as `_`), a schema that doesn't compile, a duplicate
`type@v`, v0, and a version past 1 without an upcast. The upcast is the
extension's Starlark (`extension_event_types::starlark_upcast`:
`transform({from_v, payload})`, sandboxed with the command-script budget,
no host); its output is validated against the newest schema. What the
running vocabulary registered is restated, per `type@v`, into
`event_type_contract` (V144; `v_event_type`), which keeps a declared
type's first schema as its contract and a removed extension's types as
`registered = 0` (P8.D3, extensions.md "Event types"). `append_tx`
refuses an unregistered `type@v` or a payload that fails its schema
(`DomainError::Invalid`, naming the JSON path) before writing. Core
producers build envelopes with `Envelope::typed::<T>(source, &payload)`
so the shape is checked by the compiler too. **A payload owns its enums**
(tsk510): the golden schema is a published contract, so an event uses its
own vocabulary type (`ToolDecision`, `LoggedAgentStatus`, `Harness`) with
public doc comments, converted from the internal type at the producer —
never an internal enum whose doc comments and variants (a derived-only
`stalled`) would leak into the contract. Every fixture under
`tests/fixtures/events/` sets every optional field its type has.

**Golden schemas.** Each core `type@v`'s JSON Schema is checked in at
`crates/oxplow-domain/schemas/events/<type>@<v>.json`;
`tests/event_schemas.rs` regenerates it from the Rust type and fails on
any difference. A published event shape is a contract, so a change is a
new version — `V + 1` with an `upcast` from the old shape — never an edit
of the old file. `OXPLOW_BLESS=1` writes a golden for a type that has
never shipped. Every `type@v` also has an example payload at
`tests/fixtures/events/<type>@<v>.json`; the test validates each at its
own version and, upcast through the chain (`upcast_to_latest`), at the
newest, so a consumer only ever reads the newest shape. **The pump
delivers the newest shape** (P3.1): `at_latest` in
`crates/oxplow-app/src/event_pump.rs` upcasts each row before any
handler, sync or async, and before a dead-letter retry, so a row logged
at an older version still reaches a consumer written against the current
one; a row that can't be upcast dead-letters. A type the running
vocabulary doesn't know at all (its extension was removed) passes through
at its logged version instead (P8.D2): there is no newer shape to carry
it to, and the row was valid when it was appended. The first versioned type is
`agent.turn.ended@2` (adds `transcript_path?`, `usage?`); its v1 stays
registered (`AgentTurnEndedAtV1`) and upcasts unchanged. A superseded
version's Rust doc comment is part of its published schema — don't edit
it.

**`event_content` (V102, P3.2)** holds large or sensitive event bodies —
tool input and output, prompts — by the xxh3-128 hex of their bytes (the
blob store's hash, so refs are uniform): `hash PK, namespace, bytes BLOB,
size, created_at`, STRICT. A payload carries `ContentRef { hash, size,
truncated }`. A JSON body is stored by `put_json_tx` (tsk509): serialized
canonically (object keys sorted at every depth, so equal bodies share a
hash whatever order their keys arrived in) and cut to 256 KiB on a
character boundary — `size` is the whole body's, `truncated` says only the
first part was kept. It is read through `oxplow_app::event_bodies::read`,
by **event id and body** (`input` / `output`), never by bare hash: the
person's IPC read sees any stream, the MCP `read_event_content` only the
caller's own stream's events, and both return at most 64 KiB of text with
`truncated`. `list_agent_events` returns at most 1,000 rows. The retention
sweep deletes a body after its namespace's window and the event stays. It is separate from the snapshot blob store because the
retention differs (and the blob store's GC only knows snapshot rows).
`v_event_content` exposes everything but the bytes. `event_log` gained
`payload_expired_at`: `payload` is NOT NULL, so payload expiry writes `'{}'`
and stamps it. The sweep is `oxplow_db::event_retention::sweep` (P3.11,
ten minutes after boot, then daily; batches of 5,000 rows per transaction
through the partial index `event_log_live_payload (type, at) WHERE
payload_expired_at IS NULL`, V103, read as a `type` range because a `LIKE`
can't use a BINARY index; windows in `oxplow_domain::events::retention`'s
`CORE_WINDOWS`: `agent` 30 d payload / 14 d body,
`test`, `code`, `collector` and `effect` 90 / 30, `ui` (op errors the
person saw, tsk1072) 30 / 14, and every namespace core
doesn't own — a plugin's, found by skipping through that index one
namespace per probe, and in `event_content` — `PLUGIN_DEFAULT` 30 / 14
(P7.B7), or the shorter window its extension declares
(`event_types.retention`, P8.D5: kept in `plugin_event_retention`, V145,
restated by the vocabulary reactor, longer than the default refused by
`check_declared` at load, and **kept when the extension is unloaded** —
its rows stay under the window it promised; an extension present without
a window drops back to the default — unless a declaring extension holds
the same namespace, which a namesake that declares nothing never speaks
for, tsk796); core's state namespaces are kept). **A project sets its
own windows** with the person-only key `eventRetention: { <namespace>:
{ payloadDays, contentDays } }` (tsk947): each sweep reads it, a core
namespace's replaces the default, a plugin namespace's is capped at the
plugin's window (`RetentionWindow::at_most`), and core state is refused
at load (`is_kept_whole`). A window is bounded (`window_problem`,
tsk985): at least 7 days for a core namespace oxplow reads back (the
agent policy reads a turn's tool payloads), at least a day for a
plugin's, at most `MAX_DAYS` (36,500 — beyond about 127,000 days the
cutoff arithmetic used to wrap into the future and wipe the namespace),
and its `contentDays` no longer than its `payloadDays`.
`Timestamp::from_unix_ms` computes in `i128`, and the sweep clamps a
cutoff to `MAX_DAYS` whatever reaches it. A window naming a namespace
nothing logs is kept — a plugin may be installed later — and reported
(`SweepReport.unused`, a warning in the log). **An expired event is
history only** (tsk501): `StoredEvent.payload_expired_at` carries the
stamp; the pump checkpoints past it without calling any consumer (a new or
renamed consumer replaying the log never sees `{}`); `retry_dead_letter`
refuses one; the sweep marks the pending dead letters of expired events
`discarded`; the status derivation ignores it. The activity log shows it
as "(details expired)" from the field, not from an empty payload.

**Projections of agent events (V102).** `agent_tool_call` gained `turn_id`
and `event_id` (`UNIQUE WHERE NOT NULL`): a row is the projection of one
`agent.tool.finished`, written with `tool_call_store::record_tx` (`ON
CONFLICT DO NOTHING` — a redelivered event writes nothing). `decision` and
`claim` gained `turn_id`. Every view over these (`v_tool_call`,
`v_token_usage`, `v_agent_nudge`, `v_decision`, `v_claim`) exposes
`turn_id`.

**The contract is `append_tx(&Connection, &Vocabulary, &Envelope)
-> seq`** (`Vocabulary::validate_envelope` checks the type, the payload
and every subject ref against the same snapshot), composed inside the producer's `Database::transaction` closure. The async
`SqliteEventLogStore::append` opens a transaction of its own and is for
activity with no state write (a tool call, a lens view). Reads:
`read_after_tx(after_seq, limit)` (oldest first — the pump's cursor),
`get_tx(id)`.

**Delivery is at least once** — the `EventPump`
(`crates/oxplow-app/src/event_pump.rs`, P1.7, tsk409). A consumer is
`trait EventConsumer { name, handles(type), handle(&Connection,
&StoredEvent) }`; the pump reads each consumer's `seq > checkpoint` in
batches and, per event, runs the handler under a SAVEPOINT and advances
the checkpoint in **one transaction**, so a consumer's writes and its
position never disagree. A handler that fails (or panics) has its writes
rolled back to the savepoint and the event is parked with
`dead_letter_tx(consumer, seq, error)` in that same transaction — a repeat
failure of the same `(consumer, seq)` bumps `attempts` rather than adding
a row, and reopens a `retried`/`discarded` letter — and the checkpoint
still advances, so one poison event never stalls the pump. Events a
consumer doesn't `handle` are skipped but checkpointed. Renaming a
consumer restarts it from the beginning of the log. `Services.event_pump`
is spawned by `boot.rs` (catches up on boot, then runs on `wake()` — which
producers call after their commit — or every 5s); `TaskService` wakes it
after a transition. A handler error that is retryable (`DomainError::
Busy`) is not a poison event: the delivery transaction fails and retries,
and if the database stays busy the event waits, checkpoint unmoved
(tsk437 review).

**The consumers (after P3).** Every reaction to the log, by checkpoint
name. "Boot" consumers are registered by `Services::boot` and run wherever
`Services` does (tests included); "boot.rs" ones need an `Arc<Services>`
and are registered by `crate::boot` — a test that wants them calls their
`register` itself.

| Consumer | Kind | On | Does | Registered |
|---|---|---|---|---|
| `page_ref.work_item` | sync | `work_item.created` / `edited` | re-projects a task's body-mention edges | boot |
| `tool_call.project` | sync | `agent.tool.finished` | the `agent_tool_call` row (one per event) | boot |
| `wiki.attribution` | sync | `agent.tool.finished` | marks an edited, indexed wiki page touched by the thread | boot |
| `ui.push` | sync | `snapshot.taken` (that recorded files), `vcs.head.moved` | the renderer's `SnapshotTaken` on the UI bus (P7.B6) | boot |
| `metrics.entity_states` | async | `work_item.*`, `snapshot.taken`, `collector.synced` | re-captures state entity metrics, throttled per metric (P7.B6) | boot |
| `config.workspace_filter` | async | `config.changed` (`generated`) | the snapshot captures' filter | boot |
| `effort.lifecycle` | async | `effort.opened` / `closed` | snapshot pins, metrics; logs `effort.finished` | boot |
| `effort.claim` | async | `agent.tool.finished` | claims an edited file for the effort it was edited in | boot |
| `thread.checkpoint` | async | `snapshot.taken` (`turn_end`) | logs `thread.checkpoint` (changed since the turn began, writing tool count) | `Services::new` |
| `effort.policy` | async | `work_item.state_changed`, `effort.linked` / `opened`, `thread.checkpoint` | the default effort policy: opens, links, closes efforts through `effort.*` (`.context/work-tracking.md`) | `Services::new` |
| `effort.observe` | async (after `effort.policy`) | `thread.checkpoint` | records the turn's changed files as the effort's `observed` files, from the later of the turn's start and the effort's | `Services::new` |
| `effort.landing` | async | `vcs.commit.indexed` | logs `effort.landed` for each open effort on the stream the commit holds (git-integration.md "Commit indexer") | boot.rs |
| `collection` | async | `agent.tool.finished` (Bash) | test / analysis / coverage captures, `test.*` events, nudges | boot |
| `advisories.post_tool` | async | `agent.tool.finished` | post-tool-use advisories, persisted as nudges | boot |
| `advisories.turn_end` | async (after `effort.observe`) | `thread.checkpoint` | turn-end advisories (hints) for the effort holding the turn's end, persisted as undelivered nudges for the next prompt | `Services::new` |
| `token_usage.turns` | async | `agent.turn.ended` | a turn's token rows (transcript tail or reported counts) | boot |
| `effort.evidence` / `effort.decisions` / `effort.commits` | async | `effort.finished` | evidence rows, inferred decisions, the commits that hold its work linked to its task (`commit_links`, tsk1035) | boot.rs |
| `search.index` | async | `snapshot.taken` | the search index's file contents (the other kinds are assets, `kind_search.rs`) | boot.rs |
| `config.extensions` / `config.providers` / `config.metrics` | async | `config.changed` (`extensions`; `extensionInstances`, `activeProviders`; any key) | after the in-memory swap: the extension catalog's change signal; the provider registry reconciles; the metric catalog reseeds (P7.B6) | boot.rs |
| `extension_models.entities` | async | `collector.synced` | the extension models compile again (a new entity may let one) (P7.B6) | boot.rs |
| `change.analyze` | async | `snapshot.taken` (that recorded files), `vcs.head.moved` | re-analyzes the stream's working change and open efforts' changes, skipping an event a newer one supersedes; dead-letters a failure naming the stream (P7.B4) | boot.rs |
| `plugin.repair` | async | `plugin.disabled` | files the contribution's repair work item on the active provider as the system, or comments on its open one; records it in `plugin_health.repair_item` (P7.C2) | boot.rs |
| `collector.triggers` | async | what enabled collectors' `on:` name (never `collector.synced`) | runs each matching collector for the event, once per event (`collector_run.last_event_id`): an entity collector's rows, or a fact collector through the fact engine (`snapshot.taken`: a delta collector only when the take recorded files, a whole-tree one on every take; `effort.finished` over the effort's end snapshot; anything else over the stream's latest snapshot) — plus `collector_run` and `collector.synced@1` (P7.B3; replaced `effort.gauges` and the metrics bus `SnapshotTaken` arm) | boot.rs |

**Async consumers (P2.6.2, tsk454).** `trait AsyncEventConsumer { name,
after() -> Vec<String>, handles(type), async handle(&StoredEvent) }`
(`after` is owned so a consumer can derive it from declarations, as
`collector.triggers` does; `EventPump::consumer_names` lists what it may
name) is for work that can't run
in a SQLite transaction (take a snapshot, call a service). The pump runs
the handler outside any transaction — on its own task, so a panic is a
failure — and checkpoints in a transaction after it returns: a crash
mid-work re-delivers, so handlers must be idempotent. `Busy` leaves the
checkpoint (the next run retries; later events for that consumer wait
behind it, `PumpReport.deferred`); any other failure or a panic is a dead
letter plus checkpoint, and `retry_dead_letter` re-runs the async handler.
Registered with `register_async` after the services they drive exist. Each
async consumer has its own lock, wake-up and loop (P2.6b), so a slow one —
a model call — delays only itself: `spawn` starts one loop for the sync
consumers and one per async consumer, `wake()` wakes them all, and a loop
that moved its checkpoint wakes the others (its handler may have logged
what they consume). **Ordering (tsk506):** an async consumer may declare
`after()` — consumers whose effect it reads; an event reaches it only once
each of theirs has checkpointed past it (until then the delivery is
deferred, checkpoint unmoved, later events behind it), so no reactor reads
another's half-written result for the same event. `after_for(event_type)`
narrows that to one event's predecessors (default: `after()`);
`collector.triggers` uses it so a collector's `after:` holds up only the
events that trigger it, and `after()` still orders consumers in `settle`.
`EventPump::settle(&[names], timeout)` spawns a catch-up of just the named
consumers — side by side within a level, a consumer in a level after those
it runs `after` — and waits for it, for callers whose answer needs their
effect (`TaskService` settles `effort.lifecycle`).

The one async consumer so far is **`effort.lifecycle`**
(`crates/oxplow-app/src/effort_lifecycle.rs`), on `effort.opened` /
`effort.closed`: `TaskService::on_effort_opened` waits for the startup
sweep, takes the `effort_start` snapshot and pins it (skipped when already
pinned). An open delivered after its close — nothing settled in between —
pins the stream's last snapshot at or before `started_at` instead
(`latest_snapshot_at_or_before`): a capture then would include the
effort's own work. `on_effort_closed` takes and pins the
`effort_end` snapshot (falling back to the start pin), projects the
lifecycle metrics, then logs
**`effort.finished@2 { effort, work_item?, end_snapshot? }`**
(caused by the `effort.closed`, dedupe key `effort.finished:<effort>` so a
re-delivery's second append is a no-op). `effort` is the ref
(`effort:eff12`), never a bare id: every consumer reads it through
`effort_lifecycle::effort_of` (over `refs::build::effort_of_ref`), so no
consumer parses it its own way. Re-delivery is safe: pins are stamped only
while NULL (`set_start_snapshot` / `set_end_snapshot`), the lifecycle
metrics stop when the effort already has its `effort-lifecycle` capture,
`effort.finished` is deduped, and
`retry_dead_letter` runs under the consumer's lock.

**Effort reactors** (`crates/oxplow-app/src/effort_reactors.rs`, P2.6b)
consume `effort.finished`, each its own async consumer registered at boot:
`effort.evidence` (rebuild evidence rows), `effort.decisions` (infer
decisions — a model call; failures logged). (`effort.gauges` is gone:
`{ on: [effort.finished] }` fact collectors run from `collector.triggers`.)
They hold `Services` weakly (the pump is part
of it).
`TaskService::update` / `create`, `effort.report`, and MCP `run_command`
after any write call `settle` on `effort.lifecycle` (up to 10 min; a
start baseline on a huge repo waits for the startup sweep) so a report
lands on the effort a close just before it closed and a batch's opens
pin before its closes; the consumer holds
`TaskService::without_event_pump()` so there's no reference cycle.
Letters are `pending | retried | discarded`; the
person's moves are `retry_dead_letter(id)` (re-runs the consumer now;
`retried` on success, else `pending` with the new error; refused unless
the letter is `pending`) and `discard_dead_letter(id)` — **RPC only**
(parity `ui`): a person decides a letter's fate. `list_dead_letters(all?)`
is RPC + MCP. Also
`v_event`, `v_event_dead_letter`, `v_event_checkpoint` in the semantic
layer (V94). Nothing is skipped silently.

**Consumers so far.** `PageRefWorkItemConsumer` (`page_ref.work_item`,
`crates/oxplow-app/src/page_ref_consumers.rs`) re-projects a task's
body-mention `page_ref` edges on `work_item.transitioned` through the
`replace_source_for_ref_types_tx` core — the projection the transition
used to run post-commit, now checkpointed and dead-lettered like any
other consumer. Task insert/update still project inline until they log
events of their own.

`command_audit` (who ran which command, the input, outcome, the undo as
`inverse_json`, and `undone_by`) is written by the command bus through
`command_audit_store::insert_tx` / `set_event_id_tx` / `mark_undone_tx`
inside the run's transaction; `SqliteCommandAuditStore` reads it. See
[commands.md](./commands.md). V147 rebuilt it (an `effect` actor kind in
the CHECK) with `INSERT … SELECT` into the new AUTOINCREMENT table, which
set `sqlite_sequence` to `max(id)` — any higher high-water mark was lost.
Harmless while nothing deletes audit rows; a future rebuild that must
keep it copies the old `sqlite_sequence` row too.

**Status is written only by the status core.** `update_task_tx` writes
a task's fields, never `status` / `completed_at`; `write_status_tx`
(inside `apply_status_tx`) is their only writer, and a status change
reads the committed status inside its transaction (`set_status_tx`), so a
copy read before a concurrent status change can't revert it (review of
P2.6, tsk460). `update_with_status_tx` writes fields, logs
`work_item.edited@1 { work_item, fields }` when title / description /
priority / parent / thread changed (`move_task` logs `thread` too, anchored
to the destination), then moves the status — the core of
`work_item.update` and `TaskService::update`. Filing a task
(`insert_logged_tx`, the core of `work_item.create` and
`TaskService::create`) logs **`work_item.created@1 { work_item, status }`**;
filing into a status is a creation
with that status, not a `ready →` transition. Every provider's state
change also logs core's `work_item.state_changed@1 { work_item, to }`
([work-items.md](./work-items.md)). The `page_ref.work_item`
pump consumer projects a task's body-mention edges on `work_item.created`
and re-projects them on `work_item.edited` (it used to follow
`work_item.transitioned`, whose status change moves no body edge).

**Producers so far.** Every task status change (P2.6.3, tsk455 — not
only in_progress crossings, thread-less tasks too) goes through one
core, `task_store::apply_status_tx`, via `update_logged_tx` (an edited
row), `insert_logged_tx` (filing straight into a status logs it as a
change from `ready`) or `set_status_tx` (read-modify-write, the core of
the `work_item.transition` command). It appends `work_item.transitioned@1`
in the same transaction as the status flip — subject
`work_item:oxplow:tskN`, anchors `stream` (looked up from the thread
inside the transaction; none for a backlog task) / `thread`, payload
`{ work_item, from, to }`. Run as
`work_item.transition`, its source is the actor and its cause the run's
`command.executed`; from `TaskService` directly it is
`system:task_service`. A same-status re-issue logs
nothing; a failed transition rolls the row back with the rest. It sets
no dedupe key: a transactional producer's retry has already rolled back,
so keys are for at-least-once producers. `TaskService` keeps its
post-commit `TasksChanged` broadcast as the UI wake-up.

**Effort events.** `effort_store::start_tx` and `finish_tx` — the only
cores that open or close an effort — append
`effort.opened@2 { effort, work_item?, thread, start_snapshot? }` and
`effort.closed@2 { effort, work_item?, end_snapshot?, closed_by? }`
themselves, so every path logs: the `effort.*` commands, `thread.close`,
`stream.archive`, and the async `start` / `finish` / `close`. They are
v2 only (V8 moved logged v1 rows to v2 and stripped the old
`retroactive` flag). Subject `[effort:effN, <work_item ref>]`; anchors
stream / thread / effort (+ `snapshot` when pinned at open or close). A
`finish_tx` on an already-closed effort changes nothing and logs nothing
(`UPDATE … RETURNING`). Cores take an `EventCtx { schemas, source, cause
}` (`event_log_store`): `EventCtx::system(schemas, "task_service")` for a
system writer; a bus command will pass its actor's source and its
`command.executed` id as cause. `anchors_for_thread_tx` gives the thread
+ its stream for any event about work on a thread.

V93 also dropped `task_event` (a per-task audit table nothing had written
since V1) and wiped `page_visit` (its rows carried pre-canonical tab ids).

### `wiki_page_thread_update` — wiki-note thread-update tracking (table in `crates/oxplow-db/migrations/` + helpers in `crates/oxplow-db/src/wiki_page_thread_updates.rs`)

Per-thread attribution side table for wiki page edits. Notes themselves
are global (one body per slug, shared across all threads/streams), but
the rail's "Finished" list filters by which thread last touched each
note — mirrors how task efforts attribute via
`effort.thread_id`.

Columns: `slug, thread_id, updated_at`. PK `(slug, thread_id)` so
repeated edits in the same thread upsert in place. Index on
`(thread_id, updated_at DESC)` drives the rail query.

One writer: the `wiki.attribution` pump consumer
(`knowledge::WikiAttribution`), from `knowledge.page.written` events
carrying a thread anchor — a command run by an agent's thread
([knowledge.md](./knowledge.md)).

A hand edit (the watcher) and a person's save from the editor carry no
thread, so they mark nothing — guessing would be worse than abstaining.
Deleting a page drops its attribution rows (`slug` references
`wiki_page` `ON DELETE CASCADE`), so removed pages don't linger on the
rail under their last author.

### `usage_event` — `UsageStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Generic (kind, key) usage tracking. Append-only event log with columns
`stream_id (nullable), thread_id (nullable), kind, key, event,
occurred_at`. Aggregates (most-recent, most-frequent, currently-open)
are derived by query rather than stored, so adding a new "kind"
(editor file, task, future surfaces) needs no schema change.
Indexes: `(kind, key, occurred_at DESC)`, `(stream_id, kind,
occurred_at DESC)`, `(thread_id, kind, occurred_at DESC)`. Both scopes
are recorded simultaneously — `stream_id` is the workspace tab,
`thread_id` is the active thread within it — so consumers can roll
up by either dimension or intersect them.

The store coalesces rapid repeats: if the most recent matching
`(kind, key, event, stream_id)` row is younger than `coalesceMs`
(default 30s), `record()` bumps its `occurred_at` instead of inserting
a new row. This keeps history clean when a user re-selects the same
target several times in quick succession.

Current write hookpoints (all in `apps/desktop/src/App.tsx`, all pass both
`streamId` and `threadId`):

- `wiki-note` — `handleOpenNote` records a visit when a note becomes
  the active center tab. Drives the Notes pane's "Recently visited"
  section via `listRecentUsage({ kind: "wiki-note", … })`.
- `editor-file` — `handleOpenFile` records a visit when a file
  becomes the active center tab. Not yet surfaced in UI; collected
  for future "recent files" / "files this thread cares about" views.
- `task` — `handleRequestEditTask` records a visit when the
  user opens the edit modal. Not yet surfaced.

UI surfaces consume via `subscribeUsageEvents(listener, { kind })` to
refresh on cross-window visits without polling.

### `code_quality_scan` + `code_quality_finding` — `CodeQualityStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Deterministic, language-agnostic findings produced in-process by
two tree-sitter analyzers (`metrics` and `duplication`). Two
tables, one store, one runtime method (`runCodeQualityScan`). The
store doesn't run analyzers itself — the runtime calls
`crates/oxplow-app/src/code_quality_runner.rs` and hands normalized
findings back via `completeScan`. The legacy values `'lizard'` and
`'jscpd'` are migrated to `'metrics'` / `'duplication'` by V8.

`code_quality_scan` rows: `id, stream_id, tool ('metrics' |
'duplication'), scope ('codebase' | 'diff'), base_ref (nullable, set
when scope = 'diff'), status ('running' | 'completed' | 'failed'),
error_message, started_at, completed_at`. One row per analyzer
invocation per `(stream, tool, scope)` combination. Index on
`(stream_id, tool, started_at DESC)` makes "latest scan per tool"
cheap.

`code_quality_finding` rows: `id, scan_id, path, start_line, end_line,
kind ('complexity' | 'function-length' | 'parameter-count' |
'duplicate-block'), metric_value (REAL), extra_json`. The metrics
analyzer emits three findings per function (one per metric kind)
with `extra.functionName` for grouping. The duplication analyzer
emits two findings per duplicate-pair (one per side) with
`extra.peerPath` / `extra.peerStartLine` / `extra.peerEndLine` so
the UI can render "duplicates X lines from Y:Lstart-Lend" without
re-querying.

Retention is store-driven, not schema-driven: each `completeScan`
prunes old scans for the same `(stream, tool, scope)` triple beyond
`keepLast` (default 10), deleting their findings in the same
transaction. Different scopes retain independently — running the
diff scan many times doesn't evict the codebase scan.

`listLatestFindings({ streamId, tool?, paths? })` joins on the most
recent `completed` scan per `(stream, tool, scope)`, ignoring
running/failed scans entirely so the panel never shows partial
results. The `paths` filter (used by the Diff vs base tab) intersects
findings against `listBranchChanges`'s file list at query time, so
findings persisted by a codebase scan can also drive a focused
"changed files only" view without re-running.

The store publishes `code-quality.scanned` events on start /
complete / fail; `CodeQualityPanel` (`apps/desktop/src/components/CodeQuality/`)
subscribes via `subscribeCodeQualityEvents(streamId, fn)` and
refetches.

### `effort_observation` — **RETIRED** (dropped in migration `V39__drop_effort_observation.sql`, tsk215)

The `effort_observation` table + `SqliteEffortObservationStore` are **gone**.
Coverage / test / static-analysis facts live in the **fact substrate**
(`fact` rows under `metric_capture`, see `.context/metrics.md`); the rich
detail that used to live in `payload_json` (test suite/case tree, coverage
per-file line-sets, analysis payload) rides verbatim in
`metric_capture.detail_json` (the `{"kind": …, "payload": …}` envelope, T-E1).
The `effort_evidence` asset rebuilds each effort's rows from its own run
captures (`metric_capture.effort_id`, trigger `on-report`;
`CollectionService::effort_observations_from_metrics`) and stores them
in `effort_observation_row` (V80, read as `v_effort_observation`); the panel and
MCP `list_effort_observations` read those. `effort_evidence_state` records the
signature each effort's rows were computed from — the count and newest id of
its run captures and its file count (`effort_evidence_store.rs` `SIG`; V7
restamped stored ones) — so a closed effort whose runs or files move is
recomputed. `EffortObservation`
(`effort_evidence_store.rs`) is that row (tsk862). The `provenance`/`source` trust spine and the `observed`/`asserted`
distinction carry on every capture (see `.context/metrics.md`).

### `metric_definition`/`metric_run`/`metric_sample`/`metric_finding` (+ `metric_dimension`/`metric_subject`) — **RETIRED** (dropped in `V49__drop_legacy_metric_tables.sql`, T-E3/tsk50)

The V38 metric cluster + `metric_store.rs` are **gone**. The fact substrate
(`measure`/`dimension`/`metric_spec`/`metric_capture`/`fact`, V43+) is the sole
metric store: producers write facts on captures, metrics are SPECS aggregated
at read time by `MetricEngine`, and the run identity for attribution is the
capture id. See `.context/metrics.md`. A capture names its stream, thread,
producing effort (`effort_id`) and — V157 — the agent turn it was measured
in (`turn_id`, FK `agent_turn`, SET NULL; NULL when no turn produced it).

> **Timestamps are fixed-width by construction (tsk243 → tsk387/tsk419).**
> Timestamps are RFC 3339 TEXT that SQLite compares lexicographically. The
> `time` crate's default formatter trims trailing fractional zeros, and `'Z'`
> sorts after every digit, so a trimmed `…20.5Z` sorted *after* `…20.500001Z`
> and a whole-second `…20Z` after everything in its second — inverting
> `ORDER BY …_at` and window comparisons. The first fix (`canonical_ts`, used
> by six stores) left the other twelve trimmed; the flaky
> `thread_grows_and_orders_oldest_first` was two comment messages inside one
> trimmed prefix. Now `Timestamp` itself serializes to the fixed-width
> `YYYY-MM-DDTHH:MM:SS.ffffffZ` (27 chars; `Timestamp::to_text`,
> `oxplow-domain/src/time.rs`), every store writes through the one
> `database::ts_to_string` / `string_to_ts` pair, and **V95** normalized every
> `*_at` / `at` TEXT column that existed at V94 (a test in `database.rs` pins
> that the migration names every such column). Parsing still accepts any RFC
> 3339 text. A new store needs no special handling — there is no other way to
> turn a `Timestamp` into text.

### `metric_cube` + `metric_live_fact` + `metric_cube_state` — the aggregate cube (`crates/oxplow-db/src/fact_store.rs`, migrations `V62__metric_cube.sql` tsk96 + `V63__branch_aware_cube.sql` tsk97)

**Derived tables — the only ones in the schema that hold no source of truth.**
Every row is 100% reconstructible from `fact`; deleting all three costs speed and
nothing else. They materialize the partial-scope fold so a sparkline is a GROUP BY
over a few hundred pre-folded rows instead of a replay over every fact (measured:
9.26s → 70ms per metrics-page refresh).

- **`metric_cube`** — the aggregate fact table, grain `(measure, capture, producer,
  promoted dims)`, holding the decomposable components
  `count/sum/min/max/numerator/denominator`. Sits beside `fact` sharing the same
  `metric_capture` dimension — ordinary star-schema aggregate navigation.
- **`metric_live_fact`** — the fold's live state made durable, keyed by
  `(measure, stream, branch, producer, subject_key)` — one partition per branch,
  mirroring the fact fold (V63); a new branch's partition is SEEDED from the
  history visible at its first capture ("visible" = the ancestry rule,
  tsk102/V65 — see `.context/metrics.md`). `branch` is `''` for a branch-less
  capture (a WITHOUT ROWID PK can't hold NULL; the `''` mapping lives in the
  store layer only). Sized by live subjects, not history.
- **`metric_cube_state`** — the watermark, per `(measure, stream, branch)`
  (V63). Distinguishes "state was legitimately empty here" from "not cubed yet";
  without it a materialized read reports 0 instead of admitting it doesn't know.
  The stream's watermark is the MAX across its branch rows, and a row's
  existence doubles as the branch-seeded marker.

**Rules before touching these:** never let a read take *data* from the cube that
the facts don't have; never aggregate coarser than a capture (it's the floor that
keeps snapshot/effort tie-back working); `dimension.promoted` defines the grain, so
promoting a dim is a **rebuild**, not a schema change. **Anything that deletes
captures or facts must invalidate the affected stream's cube in the same
transaction** — cube rows are frozen at build time and don't cascade (tsk100;
`prune_dominated_tree_captures` is the one such caller today, and it invalidates
only when it actually dropped something, since it runs on every boot). Written outside the
fact-insert transaction, which is safe only because whole-capture replay is
idempotent and the watermark advances atomically with the rows. Full rationale +
the read's eligibility rules: **`.context/metrics.md`**.


### `test_case_stat` — each test's summary (`V139__test_case_stat.sql`, tsk733)

One row per `(stream, branch, producer, subject)`, STRICT: last status
and duration, the last duration written as a fact (`recorded_ms`, what the
change-only tolerance compares to), max and running total duration,
runs, failures, flips, first / last seen, last failed / passed, the
latest run (`last_run_id`, SET NULL with its capture). Written only by
`SqliteFactStore::record_test_run`, in the transaction that records the
run and its change-only per-case facts. Published as `v_test_case_stat`
(`mean_ms` derived). Cascades with its stream.
### `agent_nudge` — `SqliteAgentNudgeStore` (`crates/oxplow-db/src/agent_nudge_store.rs`, migration `V33__agent_nudge.sql`)

The persisted record of the informational **nudges** oxplow surfaces to the
agent from the PostToolUse hook (`crates/oxplow-app/src/collection.rs`
`on_post_tool_use`) — the report-less-test-run nudge and post-tool-use
advisories.
Previously fully ephemeral (returned as `additionalContext`, then
lost); persisting gives a reviewer/human-facing answer to "what did oxplow
tell the agent this effort." See `.context/agent-model.md` (Nudge
persistence).

Columns: `id, thread_id (NOT NULL, FK threads ON DELETE CASCADE), effort_id
(NULLABLE, FK effort ON DELETE CASCADE), kind, message, trigger
(nullable), created_at`. Indexes on `(effort_id, created_at DESC)` and
`(thread_id, created_at DESC)`.

- **`thread_id` NOT NULL, `effort_id` nullable**: every nudge fires within a
  thread; today every kind fires against the open effort, but the column is
  nullable so a future thread-scoped nudge (no open effort) has a home. The
  effort FK cascades with its `effort` when present.
- **`kind`** is open-ended (`report-less-run` | `<extension>/<advisory>` |
  `configure`, …) — adding a kind needs no migration. Retired kinds keep
  their rows: `commit-hygiene` and `oxplow-bundled/coverage-target` no
  longer fire but old rows still
  read back.
- **`trigger`** is the bash command (or commit sha) that caused the nudge.
- **V10** adds `audience` (`agent` | `person`, default `agent`): a
  person's nudge is a hint raised in Alerts, stamped `delivered_at` when
  they dismiss it (`dismiss_tx`), never taken by the agent. It also adds
  `hint_stat(thread_id FK threads CASCADE, hint, evaluated,
  last_evaluated_at, PK(thread_id, hint))`, each hint's evaluation count
  (`evaluated()`); `v_hint_stat` joins it with the nudges and mute marks.
- **V102** adds `turn_id` (FK agent_turn SET NULL), `cause`
  (the event that fired it) and `delivered_at`. `UNIQUE(cause, kind,
  coalesce(effort_id, 0)) WHERE cause IS NOT NULL` (V104, tsk510; V102 keyed
  it without the effort and dropped a second effort's nudge) makes a
  redelivered event unable to fire the same nudge twice for one effort
  (`record_tx` → `None`). **Delivery is by thread, not by
  cause:** `take_for_agent(thread, budget)` returns the thread's agent
  nudges with no `delivered_at` (oxplow's own first, oldest first, up to
  a character budget), and stamps them in the same transaction —
  so a nudge whose reactor finishes after its hook's response went out
  reaches the agent on the thread's next hook instead of being lost. Rows
  from before V102 were stamped delivered.
- **One-shot marks are durable** (V102; per thread since V9):
  `once_mark(thread_id FK threads CASCADE, effort_id FK effort CASCADE
  NULL, mark, fired_at)`, unique on `(thread_id, coalesce(effort_id, 0),
  mark)` — a mark is kept in an effort, or in the thread when
  `effort_id` is NULL (`OnceScope`). It holds
  `report-less-run`, `<extension>/<advisory>` and
  `<extension>/<advisory>#<row key>`. `claim_once_tx` is the insert (`true`
  the first time); `has_fired` reads it. It replaces the in-memory sets
  (`CollectionService::nudged_efforts`, `AdvisoryRunner.fired`), so a
  restart no longer repeats guidance. A mark is separate from the nudge row
  because prompt advisories fire once-per without writing a nudge. No
  store-side retention prune (nudge volume per effort is tiny).

### `agent_token_usage` + `agent_token_cursor` — `SqliteTokenUsageStore` (`crates/oxplow-db/src/token_usage_store.rs`, migrations `V35__agent_token_usage.sql`, `V36__agent_token_usage_prompt.sql`)

Per-turn agent token accounting parsed from the session transcript
(tsk104). On Stop the runtime reads `transcript_path` from the hook
payload, splits the NEW tail into one row per agent turn, and writes them
here. See `.context/agent-model.md` (Token usage capture) for the capture
flow.

`agent_token_usage` columns: `id, stream_id (NOT NULL, FK streams ON
DELETE CASCADE), thread_id (NOT NULL, FK threads ON DELETE CASCADE),
effort_id (NULLABLE, FK effort ON DELETE CASCADE), session_id,
agent_kind, model (nullable), prompt (nullable), input_tokens,
output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
message_count, provenance (CHECK `observed`), recorded_at, turn_id, cause`.
Indexes on `(effort_id, recorded_at DESC)` and `(thread_id, recorded_at
DESC)`. V102 (P3.2) added `turn_id` (the turn the tokens were spent in) and
`cause` (`UNIQUE WHERE NOT NULL`: the `agent.turn.ended` event whose own
report — ACP — carried the counts, so a redelivered event counts once).
Transcript rows leave `cause` NULL: one transcript chunk can hold several
turns, and the cursor committing with the rows is what makes their
redelivery a no-op (tsk498).
`record_batch(rows, cursor)` writes the rows and advances the session's
cursor in **one transaction**, so the cursor never passes bytes whose rows
are missing (or vice versa); `insert_tx` / `set_cursor_tx` are the cores.

- **`effort_id` nullable** for the same reason as `agent_nudge`: a Stop
  can land with no open effort, so the turn is still attributed to the
  thread. Per-effort totals only count attributed rows; per-thread totals
  include effort-less turns.
- **`provenance` is always `observed`** — oxplow read the transcript
  directly (no agent-asserted path). **`model`** is the actual per-turn
  model (e.g. `claude-opus-4-8`) so $ cost can be layered on later;
  display is tokens-only today.
- **`prompt` (nullable, tsk143)** is the human-authored user prompt that
  OPENED the turn — pure observation read from the transcript, never
  generated or sent (no new input surface). Nullable: a turn can be an
  assistant continuation with no fresh prompt at the head of the chunk.
  Stored locally like every other effort artifact (same privacy posture
  as the token counts).
- **`agent_kind`** records who ran the turn. Only Claude is parsed today;
  Codex/Opencode rows aren't written yet (their transcript formats
  differ) — the parser is pluggable per kind.

`agent_token_cursor` columns: `session_id (PK), byte_offset, updated_at`.
The persisted read offset into each session's transcript so successive
Stops only sum the new tail — and a daemon restart never re-sums
already-recorded usage. No FK (the transcript outlives any single row).

Store methods: `record`, `list_for_effort`, `totals_for_effort`,
`totals_for_thread`, `cursor`/`set_cursor`. There are no IPC reads: the UI
reads `v_token_usage` through oxplow-bundled lenses (`task-tokens`,
`thread-tokens`, `usage`), which re-run on `ModelsChanged`.

### `panel_layout` — `SqlitePanelLayoutStore` (`crates/oxplow-db/src/panel_layout_store.rs`)

V126 (P6.G1). The person's left-nav layout in this project: one row per
panel they've placed (`panel` — a core id like `core:bookmarks` or
`ext:<extension>/<id>` — `position`, `hidden`, `collapsed`). Local state,
never the repo. `get` returns the placements in order; `set` replaces
them all in one transaction. A panel the table doesn't name shows
expanded after the ones it does, and a named panel that no longer exists
is dropped (`components/Panels/panelLayout.ts` `resolveLayout`). UI-only
RPCs `get_panel_layout` / `set_panel_layout`.

### `bookmark` — `bookmark_store` (`crates/oxplow-db/src/bookmark_store.rs`)

V4. The pages the person starred, each at one scope: `thread`
(`thread_id`), `stream` (`stream_id`) or `project` (neither — the old
localStorage "global" scope; every ref is this project's anyway). A
viewer — a thread and its stream — sees its thread's, its stream's and
the project's, and a ref is bookmarked at most once across those:
`set_tx` takes it out of every scope the viewer sees before inserting, so
bookmarking at another scope moves it. Read through `v_bookmark` (the UI's
`tabs/bookmarks.ts`, a lens, the agent); every write is `bookmark.set` /
`bookmark.remove` (commands.md). They used to live in webview localStorage, where neither lenses nor the
agent could see them; those weren't carried over.

### `capability_provider` — `SqliteCapabilityStore` (`crates/oxplow-db/src/capability_store.rs`)

V128 (P6b.C2; V15 adds `title`, `source`, `available`, `chosen_by`; V16
the capability's own `capability_title`, `choosable`, `optional`).
Each capability's implementations and their features (`capability`,
`provider` — the primary key — `extension`, NULL for core's,
`features_json`, `active`). Restated whole (`reset`) by the app's
`CapabilityRegistry` (`capabilities.rs`) at boot, when the extensions
change, as an instance starts or stops, and on each reconcile. An
extension provider's `provider` is its **instance
id** (P9.B1: a provider's default instance has the provider's id, a
second instance its own — `issues_acme`), the same id its refs and
`plugin_health.contribution` carry. Published as `v_capability_provider`;
see [work-items.md](./work-items.md).

### `asset_state` — the asset runner (`crates/oxplow-app/src/assets.rs`)

V130 (P7.B1). Each asset's last recompute: `asset` (the primary key —
`metric_cube`, a materialized model's view), `computed_at`, `events_to`
(the event log's highest seq as the recompute began), `snapshot_id` (when
the asset has one), `elapsed_ms`; V143 (P8.B4) adds `mode` (`full` /
`incremental`), `watermark` and `row_count`; V151 adds `definition` (a
materialized model's SELECT, hashed — a clocked asset recorded for
another definition is due at once, tsk780). Written by the runner after
each recompute; read as `v_asset`. **`asset_failure`** (V152, tsk781):
an asset's last recompute failure — `failed_at`, `error` — written when
one fails and deleted in the transaction of the next success; `v_asset`
(v3) joins it, so an asset that never succeeded is listed too. See [semantic-layer.md](./semantic-layer.md)
"Assets".

### `collector_run` — `SqliteCollectorStore` (`crates/oxplow-db/src/collector_store.rs`)

V133 (P7.B3; replaced V75 `ext_source_state`, its rows copied). Each
collector's last run: `owner` + `id` (the primary key; the owner is the
declaring extension, `project` or `built-in`), `status` (`ok` / `error` /
`needs_approval` / `skipped` — the loop guard refused its run for an
event, V154 rebuilt the table for the CHECK), `last_run_at`, `error`, `row_counts_json` (rows per
entity after the last good run), `cursor_json` (its opaque checkpoint)
and `last_event_id` (the last trigger event it ran for; it never moves
back). A failed run keeps the last good counts and cursor.
`record_run_in` runs in the transaction that writes the run's rows and
logs its `collector.synced@1`. Read as `v_collector_run`; see
[semantic-layer.md](./semantic-layer.md) "Collectors".

### `effect_run` — `crates/oxplow-db/src/effect_run_store.rs`

V149 (P8.D10), rebuilt by V156 (P9.D4). Each **attempt** at an
extension effect's reaction to an event: `effect` (`<extension>/<id>`),
`event_id` / `event_seq`, `attempt` (from 1), `origin` (`live` — the
event's delivery; `retry` — a person's `effect.retry`; `backfill`),
`state` (`started` / `ok` / `skipped` / `proposed` / `failed`), `reason`,
`audit_id`, `proposal_id`, `started_at`, `finished_at`. `UNIQUE (effect,
event_id, attempt)`: an attempt is made once, and the reaction's state is
its latest attempt's (`latest_tx`; `v_effect_run.latest`). The command
bus writes an attempt's row in its run's transaction, or claims it
`started` first when a step runs outside it (`claim_tx` / `finish_tx`).
See [extensions.md](./extensions.md) "Effects".

### `plugin_health` — `SqlitePluginHealthStore` (`crates/oxplow-db/src/plugin_health_store.rs`)

V135 (P7.C1). Each plugin contribution's health on this machine, keyed
`plugin` (the extension) + `kind` (`provider` | `collector`) +
`contribution` (its provider's or collector's id) — V140 put `kind` in
the key, since a provider and a collector may share an id (tsk721):
`state` (`ok` / `failing` /
`disabled`), `reason`, `consecutive_failures`, `last_ok_at`,
`last_error`, `mean_ms` (a moving average), `next_due_at`, `updated_at`.
`next_due_at` is when a scheduled contribution should have run again by
— written each minute by the schedulers (`run_due_collectors` for an
approved, enabled `every:` collector; `ProviderRegistry::sync_due` for a
running instance, its earliest collector): the last run (or now, when it
runs now) plus the interval plus one scheduler tick
(`plugin_health::next_due_ms`); `NULL` for one off any schedule (manual,
`on:`, unapproved, disabled, `syncMinutes: 0`). A rate-limited instance
keeps its last plan (tsk722). The policy is `oxplow-app/src/plugin_health.rs`:
three failures in a row disable it, the row and `plugin.disabled@1` in
one transaction; `plugin.enable` clears it (`plugin.enabled@1`).
`v_plugin_health` adds `dead_letters` (pending dead letters of its
consumer `extension:<plugin>/<contribution>`, or of events whose subject
is `plugin:<plugin>`) and `fresh` (0 once `next_due_at` has passed).
V136 (P7.C2) adds `repair_item` (the repair work item's ref the
`plugin.repair` consumer filed) and `repair_seq` (the last
`plugin.disabled` it handled, so a redelivery files nothing twice);
`v_plugin_health` v2 shows `repair_item` only while that item is open.

### `provider_collector_state` — `SqliteProviderCollectorStore` (`crates/oxplow-db/src/provider_collector_store.rs`)

V129 (P7.A3). Where each provider instance's collector left off:
`instance`, `collector` (the primary key), `state_json` (the provider's
own opaque `$/state` checkpoint), `status` (`never` / `reading` / `ok` /
`error`), `error`, `last_read_at`, `records` (what reads have delivered).
`checkpoint_tx` runs in the same transaction as the
`work_item.recorded@1` events a checkpoint covers, so a read that fails
midway resumes from the last batch that landed; `finish_tx` records the
outcome. Read by `provider.sync` and Settings → Integrations; see
[providers.md](./providers.md) "Reading: collectors and sync".

### `command_proposal` — `SqliteProposalStore` (`crates/oxplow-db/src/proposal_store.rs`)

V127 (P6b.A2). A command an agent ran that needs a person's confirmation,
kept until a person decides: the command and input, who proposed it
(`actor_kind`, `actor_id`, `thread_id`, `stream_id`), the confirmation
`preview_json`, `dry_run_json` (what it would have done when proposed;
NULL when it can't be dry-run), and `decision` — `pending`, `approved`
(with the approving run's `audit_id`), `declined` or `superseded`.
`proposal_key(command, input)` is `config:<key>` for `config.set` /
`config.unset`, else the command and its input with keys sorted;
`insert_tx` marks pending rows with the same key `superseded` (with
`superseded_by`) and returns which (`Inserted { id, superseded }`, for
`command.proposed@1.supersedes`). `approve_tx` / `decline_tx` decide a
pending row once — a decided one is `Invalid`, a missing one `NotFound`.
An `External` command's approval is the claim trio instead: `claim_tx`
marks it approved with no `audit_id` before the run, `finish_claim_tx`
names the run's audit row, and `release_claim_tx` makes it pending again
when the run failed. `dry_run_json` is the `Tx` handler's `result` run
confirmed and rolled back: `config.set` / `config.unset` give `{ key,
before, after, changed }`, a composite `{ result, children: [{ name,
input, result, inverse? }] }`, any other command its own result. No
expiry. Written
inside the bus's transactions; read as `v_command_proposal` (ref
`proposal:<id>`). Events `command.proposed@1`, `command.approved@1`,
`command.declined@1`.

### `page_visit` — `PageVisitStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Append-only event log of in-app page navigations. One row per visit
recorded by `App.handleOpenPage` (skipping `agent`, `new-stream`,
`new-task`).

Rows carry canonical tab ids ([refs.md](./refs.md)); V93 wiped the
pre-canonical ones. Columns: `id`, `page_kind`, `page_id`, `visited_at` (ISO),
`duration_ms?`, `thread_id?` (added in v3 — nullable so legacy rows and
boot-screen visits with no active thread still record). Indexes on
`visited_at DESC`, `(page_kind, page_id)`, and
`(thread_id, visited_at DESC)`. Aggregates derived by query:

- `listRecent({threadId,limit,dedupeByRef,excludeKinds})` — drives the
  rail History (with `dedupeByRef`).
- `topVisited({threadId,sinceT,limit})` — most-visited rollup with
  payload+label of the latest visit per ref. Drives the rail's
  "Most visited" toggle and the Visits dashboard.
- `countByDay({refId,threadId,sinceT,untilT})` — daily bucketed
  counts for behavior-over-time charts.

Insert publishes `page-visit.changed` for renderer-side invalidation.

### The rail's Finished section

Read from the models (`workItems.readRecentlyFinished`, P6.E1b): done
tasks (`v_task.completed_at`) and the knowledge pages the thread wrote
(`v_knowledge_touch`, over `wiki_page_thread_update`; the project view
reads `v_knowledge_page.updated_at`), newest first. "Clear" is the
viewer's own gesture: a per-thread cursor in this browser
(`oxplow.finished.clearedAt`), and entries at or before it are hidden
until something newer lands. No table, no RPC.

### `comment` + `comment_message` — `SqliteCommentStore` (`crates/oxplow-db/src/comment_store.rs`, migrations `V22__comments.sql`, `V23__comment_resolved_at.sql`)

Threaded annotations the user anchors to a text selection on any page
(wiki body, code file lines, task detail). The user collects notes
across many points; the agent reviews/responds **only when asked**
(never force-triggered — no Stop-hook branch, no synthesized work
items).

`comment` is the thread anchor + metadata; `comment_message` holds
every message in the thread including the first. **Integer PK ids
(`INTEGER PRIMARY KEY AUTOINCREMENT`) — no UUIDs.**

`comment` columns: `id, stream_id (NOT NULL, FK streams ON DELETE
CASCADE), thread_id (nullable, FK threads ON DELETE SET NULL),
target_kind, target_id, quote, selectors_json, context_chain_json (V24),
referenced_refs_json (V24), intent ('note' | 'followup'), status ('open'
| 'resolved'), orphaned (0/1), author, created_at, updated_at,
last_activity_at, resolved_at (nullable, V23)`. Indexes on `(stream_id,
status, last_activity_at DESC)`, `(thread_id, last_activity_at DESC)`,
`(target_kind, target_id)`.

- **`resolved_at`** is stamped by `set_status` on `→resolved` and
  cleared on `→open`. It exists because neither `updated_at` (bumped by
  auto re-anchoring via `set_anchor`) nor `last_activity_at` (messages
  only) reliably marks *when* a comment was resolved — the Comments
  Dashboard's "resolved in the last N days" buckets read this column.
  V23 backfills pre-existing resolved rows from `updated_at`
  (best-effort).

- **Scope.** `stream_id` is the hard scope so the agent can list every
  comment in a stream regardless of which thread authored it;
  `thread_id` is the origin agent thread and is nullable (SET NULL) so
  content comments survive the thread being archived/deleted. Reads
  support per-target, per-stream, and per-thread queries; the agent's
  MCP `list_comments` queries by thread or stream (never cross-stream).
- **Anchoring is resilient, not positional.** `quote` (the selected
  text) is the durable anchor + the context handed to the agent;
  `selectors_json` (renamed from `anchor_json` in V24) is an opaque,
  schemaless per-surface hint the renderer re-validates on load and may
  rewrite through `set_anchor`. It holds a W3C-Web-Annotation selectors
  array — a TextQuoteSelector (`exact` + surrounding `prefix`/`suffix`
  context), a TextPositionSelector (`textOffset`), an `approx` flag, and
  an optional per-surface coordinate selector (Monaco line/col,
  ProseMirror from/to, terminal buffer coords). The shared resolver
  (`apps/desktop/src/components/Comments/anchor.ts`, `resolveAnchor`)
  tiers exact-quote (disambiguated by context + proximity) then a bounded
  fuzzy fallback; a fuzzy re-attach sets `approx` (shown as a dashed
  highlight + "approx" badge). Only when even fuzzy fails is the comment
  `orphaned` — still listed, no highlight. Rust never parses
  `selectors_json`, so enriching it needs no migration.
- **Typed context (V24).** Beyond the quote, a comment carries the typed
  context it was made in. `context_chain_json` is a JSON array of
  `{kind,id}` refs — the nesting of page regions the selection sat inside
  (innermost→outermost, excluding the primary target; e.g. a file row
  under a commit yields `[{commit,sha}]`). `referenced_refs_json` is
  the canonical refs found INSIDE the selection (rendered links + inline
  mentions), so highlighting a filename tells the agent it is a file.
  The store's `create` is the single source of truth for "what's
  referenced": it unions the FE-supplied refs (DOM `<a>` links inside the
  Range) with `oxplow_domain::refs::extract(quote)` over the durable
  quote, deduped on `(kind,id)`. So inline mentions (`task:42`,
  `src/x.rs`, `[[slug]]`) become typed refs even on surfaces that never
  rendered them as links, and no frontend reimplements ref parsing.
  Both reuse the canonical `page_ref` `(kind,id)` vocabulary. Unlike
  `selectors_json` these ARE parsed in Rust (into `Vec<CommentTarget>` on
  `Comment`) because the backend ref-resolver hydrates them into typed
  summaries for the agent. The store serializes/parses them; a malformed
  value degrades to an empty list rather than failing the row load.
- **Relinking.** `relink(id, quote, selectors_json)` (store) /
  `relink_comment` (IPC) is the escape hatch for a comment whose quote
  drifted past fuzzy tolerance: it rewrites BOTH the quote and the anchor
  and clears `orphaned`. Triggered from the editor's selection
  context-menu ("Relink orphaned: …"). Unlike `set_anchor` (a passive
  re-validation sync, no event) it emits a changed event.
- **"Answered" is derived, not stored.** `CommentThread::needs_response`
  (in `oxplow-domain`) returns true for an `open` `followup` whose
  latest message author isn't `"agent"`. Messages are stored
  oldest-first, so a user reply after an agent response re-opens the
  follow-up. The MCP `list_comments(status:"needs_response")` filters on
  this.

`comment_message` columns: `id, comment_id (FK comment ON DELETE
CASCADE), author (free-form, e.g. 'user' / 'agent'), body, created_at`.
Index on `(comment_id, created_at)`. `add_message` bumps the parent's
`last_activity_at`.

**Garbage collection.** `SqliteCommentStore::cleanup(retention_days)`
deletes `resolved` and `orphaned` threads whose `last_activity_at` is
older than the cutoff (`retention_days <= 0` disables pruning). The host
binary (`apps/desktop/src-tauri/src/main.rs`) runs it at boot and every
24h with a 14-day window, mirroring the snapshot cleanup loop. Open
comments are never swept.

Change fan-out: a comment's writes are the `knowledge.*` comment
commands (P8.A6); the renderer re-reads the comment models on
`ModelsChanged` — there is no comment-specific event.

### `search_entry` + `search_fts` — `SqliteSearchStore` (`crates/oxplow-db/src/search_store.rs`, migration `V25__search_index.sql`)

The unified site-wide search index (FTS5/BM25). `search_fts` is a standalone
FTS5 virtual table (`tokenize = 'porter unicode61'`, `prefix = '2 3'`) storing
`(title, body)` for every searchable entity; `search_entry` maps
`(kind, ref_id, stream_id)` → the FTS rowid so a single entity can be updated
or removed in place (a `UNIQUE` index over `(kind, ref_id, COALESCE(stream_id,''))`
enforces identity, treating global rows' `NULL` stream as `''`).

- `kind` ∈ `task | comment | note | wiki | file`, or a **searchable
  plugin ref kind** (`acme_pr`, P9.D3); `ref_id` is the task id, comment
  id, note id, wiki slug, repo-relative path, or the plugin ref's id.
  Every kind but `file` is derived from a model by an asset per kind
  (`kind_search.rs`, `restate_kind_tx`: only entries added, changed or
  gone are written, by each entry's `content_hash`, V164; tsk896); core's read `v_search_work_item` (the active work list's items,
through the interface; kind `work_item`, its id `<provider>:<id>`) /
`_comment` / `_note` / `_wiki`
  (tsk864). Files are upserted by the `search.index` pump consumer from
  `snapshot.taken` (`indexer.rs`).
- `stream_id` is `NULL` for project-global rows (wiki, a backlog task) and
  the owning stream otherwise. Search filters `stream_id = ?  OR stream_id
  IS NULL`; BM25 weights title above body (`bm25(search_fts, 5.0, 1.0)`).
- The store is a **derived cache**, never a source of truth: those two
  writers are its only ones. `purge_stream_files` is called when a stream
  is archived.
- `sanitize_query` turns arbitrary user input into a safe MATCH expression
  (each token double-quoted + `*` prefix), so junk input can't throw FTS5
  syntax errors. Exposed as the `search` IPC command + MCP tool.

## The `sort_index` queue

`tasks.sort_index` orders work in a single numeric space scoped
per thread. `runtime.reorderThreadQueue(streamId, threadId, entries)`
rewrites the values in one operation; entries are `{ id }` with
`sort_index = position`.

**Visual vs persistence order for Done.** Sections in `WorkGroupList`
render ascending by `sort_index` *except* Done, which renders
descending (newest-finished items surface on top). The underlying
`sort_index` space is still a single ascending line — the section is
only flipped at render time. When a drag-reorder persists a new order,
`finalizeReorderIds` in `plan-utils.ts` reverses each descending run
(`done`/`canceled`/`archived`) so the `reorderItems` /
`reorderThreadQueue` "sort_index = position" rule produces the intended
visual result. The drag handler passes the *effective* new status of a
row whose status is changing as part of the drop so the run detector
sees the new section membership. Dropping any item *into* Done is a
drop-to-top contract: the drag handler inserts the row at the head of
the Done bucket in visual order, and `task-store.updateItem`
bumps `sort_index` to `MAX+1` on every non-Done → Done transition, so
the two paths agree on "newest-done on top." Any new section with a
non-ascending display must either do the same reversal dance or get
its own flat list.

## Status diagrams (text)

```
task:    ready ─► in_progress ─► done ─► archived
                   ╰─────────► blocked ◄─┘
                   ╰─────────► canceled ─► archived
```

### Transitions to `in_progress` (server-side guard)

`taskstore.updateItem` rejects any direct jump into `in_progress`
from `canceled` or `archived` (those are explicit "abandoned" states
the user must re-`ready` first). All other sources are accepted:

- `ready → in_progress` (normal pickup)
- `done → in_progress` (reopen — the redo path when the user pushes
  back on shipped work)
- `blocked → in_progress` (deliberate unblock gesture)
- `in_progress → in_progress` (no-op)

`listReady` / `readWorkOptions` / `list_ready_work` filter to
`status='ready'` only — `blocked` items are never dispatchable until
un-blocked.

## Change events

Cross-store change fan-out is centralized on the typed EventBus
(`crates/oxplow-app/src/events.rs`). Stores call `EventBus::emit(...)`
with a coarse `OxplowEvent` variant (most views instead re-read on
`ModelsChanged`); subscribers refetch the affected bucket on
receipt. The bus is a `tokio::sync::broadcast` channel, so a slow
subscriber sees `RecvError::Lagged` rather than blocking publishers,
and adding/removing subscribers is lock-free.

The runtime relays each store's changes onto the typed EventBus
(`crates/oxplow-app/src/events.rs`) as `*.changed` events:

- `workspace.changed`, `git-refs.changed`, `workspace-context.changed`
- `task.changed`, `backlog.changed`, `thread.changed`
- `file-snapshot.created`, `agent-status.changed`
- `hook.recorded`, `config.changed`

UI components subscribe via `subscribeOxplowEvents()` (or scoped helpers
like `subscribeWorkspaceEvents`, `subscribeGitRefsEvents`) in
`apps/desktop/src/api.ts`. See [ipc-and-stores.md](./ipc-and-stores.md) for how to
plumb a new event end-to-end.

## UI-only state worth naming

A few UI surfaces hold non-persisted state that callers (probes,
docs, future stores) reference by name. Listed here so renames touch
the docs in one diff:

- **Files-pane filter mode** — `FilterMode = "all" | "uncommitted" |
  "branch" | "unpushed"` lives in `ProjectPanel` state and
  drives the file-tree visibility filter. The eye-icon trigger button
  is `data-testid="files-filter-toggle"`; each popover option is
  `data-testid="files-filter-option-<value>"` (e.g.
  `files-filter-option-uncommitted`). `branch` / `unpushed` auto-
  fall back to `uncommitted` if the underlying scope disappears (no
  branch base, no upstream).

## Related

- [ipc-and-stores.md](./ipc-and-stores.md) — adding new stores and IPC.
- [agent-model.md](./agent-model.md) — how the agent acts on this data.
- [git-integration.md](./git-integration.md) — `gitCommitAll` for the
  Files-panel commit button (user-driven).
