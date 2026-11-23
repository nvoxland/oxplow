# Data model


What this doc covers: the SQLite tables, their store classes, and the
assignment queues. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Job runs

What a job is and how it runs: [extensions-jobs.md](./extensions-jobs.md) "Jobs".
The lifecycle is pure in `oxplow-domain/src/job_run.rs`; the store is
`oxplow-db/src/job_run_store.rs`. V67 links each attempt to its
shared contribution run. V69 gives the five tables their `job_run` names.

- **`job_run`** — one row per enqueued job: owner (`contribution_id`, package
  version, deployment hash, contract version), operation, invoker, the run that
  asked (`queued_by_run_id`), `intent_key` (`UNIQUE (contribution_id, intent_key)`:
  one intent is one job), replay policy, `state` (`queued`, `running`,
  `succeeded`, `failed`, `cancelled`, `interrupted`), `attempt` and `fence`,
  the lease, the first cancellation request, `revision` and the private
  `request_json` (≤ 1 MiB). Every transition is a revision CAS
  (`transition_tx`); triggers keep identity, the request and the first
  cancellation time immutable, and an ended row frozen.
- **`job_run_context`** — the job's immutable provenance (the asking
  actor, anchors, subjects, cause), written with the intent; a dispatch
  without it doesn't run. No grant or credential is in it.
- **`job_run_attempt`** — one row per attempt, its odd fence kept
  after recovery; closed once (succeeded, failed, cancelled with stop
  evidence, interrupted) with its result (≤ 1 MiB). Its unique `run_id`
  references the shared run with a delete cascade. The claim begins the
  run in the same transaction as the fence update. The dispatcher
  rechecks authority before executing under that identity. Completion
  closes the run with the attempt and records measured usage. Lease
  recovery closes the shared run as `interrupted` and leaves CPU,
  memory and model costs unknown.
- **`job_run_output`** (V56) — what an attempt produced as it ran,
  `(job_id, attempt, seq)`: `kind` `text` | `tool_call` | `tool_result` |
  `usage`, `content_json` (≤ 256 KiB). Append-only; a trigger refuses rows
  for an attempt that isn't open, so a worker that lost its attempt writes
  nothing. Written by `append_output` in short transactions of its own.
- **`job_run_observation`** — late-outcome evidence: a worker that
  finished after its attempt was fenced off records what it had here, never
  on the job.

**`v_job_run`** publishes the job's state and current attempt's shared
run, with live rounds, tool calls and tokens from **`v_job_run_output`**.
The queued job gets its package and version from its declaration and
persisted version. A job's request and result are read through its extension's
`jobs.read/snapshot`. The writers are on the thin-caller clippy deny lists:
jobs change through their commands and the dispatcher.

## Extension state persistence

`extension_state` reserves storage for host-managed workflow state and
rebuildable caches, keyed by `(namespace, scope, area)` within the project DB
(V6: the extensions sharing a namespace share it, and it outlives a move
between them; it is forgotten only with the namespace's last extension).
Scope is `project` or the current host's `personal` workspace scope; area is
`state` or `cache`. These rows are private extension data, not published SQL
models, shared domain entities or configuration preferences. The schema stores
one JSON object with a positive schema version and a revision for the whole
namespace. A reset must preserve the row and increase the revision so stale
requests cannot recreate prior values. Disable and uninstall do not cascade
into deletion; there is no foreign key to the current contribution registry.

`oxplow_domain::extension_state` defines validated namespaces, stamps and
snapshots. `SqliteExtensionStateStore` reads consistent snapshots and uses
compare-and-swap on both version and revision to replace the whole map and
schema version in one retried transaction. The writer sets the schema
version either way — a later version moves it up, a rolled-back one down
(only the running deployment's commands write, so an older version is the
one running); the stamp's revision is what guards. It rejects revision overflow, invalid keys, more than 256 keys or more than 1 MiB of
serialized data or 64 levels of JSON nesting (including the namespace map),
so a valid write remains within the decoder's recursion limit. Reads reject
oversized stored bytes before decoding and reject corrupt data rather than defaulting. Conflicts carry the current stamp and do
not retry; only storage contention retries. Conditional UPDATE/INSERT also
protect the synchronous transaction helpers from overwriting a competing write.
Expected stamps are validated on both entry points. Reset uses an empty map and
retains the row; there is no physical-delete operation. Neither store errors nor
models expose state payloads. The thin callers' clippy deny lists include both
write entry points.

Extension migration code must run on a copy outside a DB transaction, then
publish with that same CAS. Host
identity/grant checks are now supplied by `oxplow-app/src/extension/extension_state.rs`:
derive the namespace from a valid declared RunContext contribution, match its
current catalog generation/version and enabled descriptor, and recheck live
authority before and after the store operation. Read inputs select only scope
and area. The write helper takes an audited TxCtx with an existing transaction
and event cause, rejects pure runs, and uses a savepoint so failed publication
checks restore state even if caught. Storage/rollback failures must abort the
outer transaction. It creates no transaction or external work of its own.
The primary-project native `state.read/snapshot` command operation is wired;
its unaudited read returns a private snapshot without publishing a SQL model.
The shared script/provider router also serves `state.read`: command composers
use their current connection, while external hosts use StateSource for a
separate owned read per call. Provider namespace ownership is captured from
its active provider declaration, independently of the invoking command.
Public write commands still require private audit/proposal input handling.
State has no migration runner; extensions transform state outside the DB and
publish with its CAS. Settings are separate: registered keys resolve atomic
local/project/global overrides and defaults through `SettingsStore`, with ordinary
values in YAML and secret values in secure storage ([settings.md](./settings.md)).
They have no DB layer stamps or schema carry-forward. The lower-level persistence
APIs grant no authority themselves. File/secure-storage mutations are not part
of a SQLite transaction.

## Storage

Project database persistence lives in `.oxplow/local.sqlite`, opened
through `Database::open` (`crates/oxplow-db/src/database.rs`). Every store is
a thin class wrapping that connection. Schema changes go through versioned
refinery migrations in `crates/oxplow-db/migrations/`, append-only; never
edit a prior version. **The history has been squashed three times.** Each
time every migration so far became one `V1__baseline.sql`: the schema
they built, dumped from `sqlite_master` (tables, then indexes, then
triggers; no views, no FTS shadow tables), with the rows they seeded. The
second squash took V1 (the first baseline) through V124; the third took
V1 through V8 when "work item" became "task" everywhere (the tables, the
columns, the event types, the URIs) and oxplow's own list's tables became
`oxtask`, `oxtask_note` and `oxtask_link`. The `V<n>` names
elsewhere in these docs are that history — which step brought a table in
— not files that exist. Existing databases were converted once, outside
the app, by a script that checked their tables, indexes and triggers equal
a fresh baseline's (views and materialized `m_v_*` tables aside) and
rewrote `refinery_schema_history` to the baseline row. With no other
users, a squash like that is the sanctioned way to drop history; editing a
migration in place is not. The one exception: a migration that has
never applied on any database (it failed inside its transaction
everywhere it ran) has no checksum stored, so it is fixed in place.
A migration that rebuilds a table to widen a CHECK moves rows to the new
value in the rebuild's `INSERT … SELECT`, never with an `UPDATE` before
it: the old table's check refuses the new value, and a fresh test
database has no rows to show it. Test such a migration on a database
migrated to the version before it (`runner().set_target(Target::Version(n))`)
that holds a row it moves.

A sweeping rename (an id, a column, a command) skips the applied
migrations and the published model golden (`fixtures/dataset_contracts.json`,
semantic-layer.md "contract"): an applied migration is checksummed by
refinery, so editing even its comment stops every database that ran it
from opening. Rewrite what the old name left in the data with a new
migration (`V26__command_ids.sql` is the example), and reword a model's
docs with a version bump.

**Migrations run with foreign keys off.** `migrate_and_compile` turns
`foreign_keys` off around the runner and, when any migration applied,
refuses to open if `PRAGMA foreign_key_check` finds a row pointing at
nothing (naming its tables). That is SQLite's procedure for rebuilding a
table (create `_new`, copy, drop, rename): with them on, dropping a table
other rows reference deletes under those rows and fails, deferred or not
(V64 rebuilds `fact_subject` under `fact.subject_id`). The pragma is a
no-op inside a transaction and refinery wraps each migration in one, so
it can't be set from a migration file. The flip side: an `ON DELETE
CASCADE` doesn't fire inside a migration, so a migration that deletes
parents deletes their children itself, or the check refuses the database.
A migration over a large table joins through an indexed key (a temp
table's `INTEGER PRIMARY KEY`), never a correlated scan: a real database
holds ~1.4M facts, and a scan per row outlasts the daemon's 60s start.

> ⚠️ Migrations are **embedded at compile time** (`refinery::embed_migrations!`,
> a proc macro cargo knows nothing about). `crates/oxplow-db/build.rs` declares
> `rerun-if-changed=migrations` so a migration-only change (pure SQL, no `.rs`
> touched — V64 was the first) actually recompiles oxplow-db. Without it, cargo
> reuses the cached crate and every downstream binary silently ships WITHOUT the
> new migration — tests stay green, the app runs, the schema change just never
> happens. Don't remove that build script.

**Migrations never create views (P4.2).** A published view is a model file
(`crates/oxplow-db/datasets/`, see [semantic-layer.md](./semantic-layer.md)
"Models"). `migrate_and_compile` drops every model's view, runs the
migrations, then compiles the models — so a migration that alters or drops
a column never has to work around a view that reads it; the model that
reads it is fixed in the same change, and the compile at open says so if
it isn't. The build script embeds `models/` the same way
(`rerun-if-changed=models`).

**The model registry** (V105, V109): `model` (one row per published view
— owner `core` or an extension, `kind` `sql` or `entity`, version,
compiled SQL), `dataset_input` (its `ref`/`source` inputs), `dataset_contract`
(the columns each view promised at each version, and since V141 its
`key_json` — the declared `key:` columns, `'[]'` for the versions
recorded before keys; outlives the view, so a changed contract or key at
the same version is refused) and `dataset_test` (the last result of each
declared test). `dataset.materialize` is `on_change`, `every <duration>`
(V142) or `incremental <column>` (V143): SQLite can't widen a CHECK in
place, so both rebuilt `dataset`, keeping `dataset_input` and `dataset_test`
aside across the cascade. `dataset.relationships_json` (V120) holds a
dataset's declared relationships, resolved (`[{from, to, to_view,
to_field}]`); V120 also dropped the recorded contracts of `v_model` and
`v_model_column`, renamed `v_dataset` and `v_field`. Readable as
`v_dataset`, `v_field`, `v_dataset_lineage`, `v_dataset_test`. V121 renamed
the tables (`model` to `dataset`, `model_input`, `model_contract` and
`model_test` likewise) and rewrote every stored `oxplow:oxplow/model?model=…`
URI to `oxplow:oxplow/dataset?dataset=…`, and a run's decision need `model` to
`dataset`.

**Changed tables per commit** (P4.6, `crates/oxplow-db/src/changes.rs`):
every pooled connection's init installs SQLite's preupdate, commit and
rollback hooks; a commit's touched tables — and which of them it
rewrote (updated or deleted in, P8.B3) — are published after each
`Database::call` / `call_mut` / `transaction` / `read`
(`Database::subscribe_changes`), and `datasets_changed` maps them to models
through `dataset_input`. A write needs nothing to be seen — the hooks see
every table, WITHOUT ROWID and a bare `DELETE` included.

**DB dispatch is gated to the pool size.** `Database::call`,
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

**Host generation publication.** `Database::transaction_with_publication` runs
on the blocking worker under the normal write permit, with an explicit
`PublicationTransaction::commit`. It lets the host retain synchronous registry
reservations through SQL commit and subsequent map publication. Success without
commit is rejected and rolled back. Only precommit storage contention retries;
postcommit errors and panics never retry. The caller must exclude readers and
leave that boundary unavailable if postcommit publication fails. Process/network
work is forbidden in this callback. A worker-owned drop guard flushes committed
table-change notifications even if the awaiting caller is cancelled or the
worker unwinds. Thin callers are denied both the helper and its commit method.

**Transactions.** One user-visible action that spans multiple writes
to the same DB must commit as ONE transaction. The pattern (see the
`transactional-boundaries-design` knowledge page): store write ops are
split into sync `*_tx(conn, …)` cores (free functions in the store's
module, taking `&rusqlite::Connection`); the async store method is a
thin `db.call` wrapper over its core; multi-write actions compose
several cores inside one `Database::transaction(f)` closure
(`crates/oxplow-db/src/database.rs`) — which owns commit/rollback and
the bounded `SQLITE_BUSY` retry (safe because a rolled-back attempt
left no trace; that's why `f` is `Fn`, borrowing what it writes rather
than consuming it). It is the **one write path**: no store opens
a transaction of its own on a raw connection — a test
(`every_store_write_runs_in_the_retried_transaction`) allows only
`database.rs` and the open-time model compile in `models.rs`. It begins **IMMEDIATE**:
the write lock is taken at BEGIN and waited for under `busy_timeout` — a
BEGIN still Busy after that wait is retried like a Busy inside the
closure — so
a read-then-write closure (the hook ingest, most `_tx` cores) can't fail
with `SQLITE_BUSY_SNAPSHOT` when another writer commits between its read
and its first write. A pure read therefore uses `Database::read` (a
DEFERRED snapshot that is always rolled back, no write lock), not
`transaction` — the request, event-by-seq, worktree and turn-signal reads
and every command that only reads (`View` / `Read`) do. **The event-log row is the one
write that belongs inside the closure**: a producer composes
`event_log_store::append_tx(tx, &envelope)` next to its state change
so the log and the state can never disagree (the outbox pattern; see
"event_log" below). Everything downstream of the log — the in-memory
`OxplowEvent` broadcast that wakes the UI, `page_ref` projections,
snapshot requests — runs AFTER commit, never inside the closure. Don't
convert existing single-op methods preemptively — extract a `_tx` core
the first time an op needs to join a transaction. Current users:
`record_take` and every `Tx` command
handler (which runs in the bus's transaction).

**Lifecycle invariant.** A task's stage never opens or closes a
turn or a request: a task is a row of the shared model, and writing it
touches the task alone. A request is read live from its turns
(`.context/work-tracking.md`); nothing opens or closes it. Boot recovery
(`crates/oxplow-app/src/recovery.rs`) only closes turns the previous run
left open.

**Error typing.** Store failures surface as typed `DomainError`
variants, not stringified blobs: `map_sql_err`
(`crates/oxplow-db/src/database.rs`) classifies rusqlite errors —
constraint violations → `Constraint`, `SQLITE_BUSY`/`SQLITE_LOCKED` →
`Busy` (the only `is_retryable()` variant), everything else →
`Storage`. `Invalid` is reserved for caller-supplied validation
failures. The IPC layer maps these to `CONSTRAINT` / `BUSY` /
`STORAGE` codes (`crates/oxplow-rpc/src/error.rs`). Don't map SQL
errors to `Invalid` in new store code.

**A caller's own reason to stop.** `Database::transaction_or` /
`rehearse_or` / `read_or` take a closure returning `TxError<A>`:
`Storage(DomainError)` (a busy one retried, as with `transaction`) or
`Aborted(A)` — the caller's own reason, rolled back, never retried and
handed back as given. The command bus ends a run's transaction this way
(`Abort::{Failed, Lost, Unchanged}`), so nothing is smuggled out of the
closure through a side channel; `transaction` / `rehearse` / `read` are
the same with no reason of their own.

## Entity ids

Every externally-visible entity id is a SQLite **autoincrement INTEGER**
rendered with a fixed **3-letter type prefix** at the application boundary,
e.g. `str5` (stream), `thr21` (thread), `tsk42` (task), `req4` (request),
`not3` (note), `trn9` (agent-turn), `ses4` (agent session),
`trm3` (terminal).
The prefix lives only in Rust (`Display`/serde) and in the TypeScript
bindings (each id type is a `string`); **DB columns and the FKs that point
at them are plain `INTEGER`**. The single source of truth is
`oxplow_domain::EntityKind` (prefix ↔ label ↔ kind) in
`crates/oxplow-domain/src/ids.rs` — core's entities; a task store's ids
are its own (oxtask's `tsk<n>`, declared to core by its `fills.id_pattern`); each id newtype (`StreamId`, `ThreadId`,
…) is an `i64` wrapper generated by one macro, with `AnyId { kind, value }`
as the type-erased form for polymorphic boundaries (the MCP
`expect_id_kind` validator). Rowid `0` is the `placeholder()` sentinel
(insert allocates a real id and returns it; AUTOINCREMENT never issues 0).
A polymorphic reference is stored as a URI ([resources.md](./resources.md)):
`page_ref.source_uri`/`target_uri` and `search_entry.uri` (and every
model's `uri` column) hold `oxplow:<namespace>/<resource>?<params>`
(`event_log.id` is a UUIDv7, also TEXT). Refs in prose and knowledge pages use the bare
self-typed sugar, e.g. `[[tsk42]]`.

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
  set or recents list. The path is resolved by
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
  this file is for a shell that didn't spawn it: a live daemon named
  here means the project is open in another app process (or a daemon
  someone started by hand), and opening it again defers to it rather than
  killing it (`oxplow_app::daemon_supervisor::{write_daemon_info,
  live_daemon}`). A daemon never outlives its app — it stops when the
  app's end of its stdin closes — so a live one isn't an orphan.

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
- a `host` (V35, `HostId`; `'local'` for every stream so far): the host its
  worktree is on. The router answers it with the path (`WorktreeRoot`,
  [vcs.md](./vcs.md) "Around the provider"); nothing takes a host
  parameter yet, and no model exposes it.
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
- a `custom_prompt` column (migration V6, nullable TEXT; read as `v_stream.prompt`, v3) — per-stream
  standing instructions appended to the agent's system prompt after the
  global `agentPromptAppend` section. Set via the `oxplow.stream.set_prompt`
  command (`apps/desktop/src/api.ts`'s `setStreamPrompt`); the stream
  list re-reads on `DatasetsChanged` naming `v_stream`.
  V6 also dropped a legacy `summary` column that was carried over from
  the TS schema and never displayed; the prompt feature briefly
  piggybacked on that slot before getting its own column.

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
by the rail's "Remove…" action. `StreamStore::list` filters
`archived_at IS NULL` so archived streams disappear from the rail
without dropping rows that history (requests, snapshots, page_visit)
references. Pre-migration rows stay NULL and remain visible. Primary
streams cannot be archived.

### `threads` — `BatchStore` (`crates/oxplow-db/src/thread_store.rs`)

Lines of the person's work *within* a stream. A thread needs no agent and
may have several: its agent slots are `agent_session`
rows. A thread is open or closed, nothing more
(`Thread::is_open()`, `closed_at` below): every open thread of a stream
shares its worktree and any of them may change it, and several agents
may share a checkout — that is the person's call (a new stream is the
isolated option). Migration V81 dropped the `status` column and the
one-active-thread-per-stream unique index. `mark` (nullable TEXT, CHECK
of `amber`, `red`, `green`, `blue`, `violet`; `Thread.mark:
Option<ThreadMark>`) is a colour on the thread's Navigator tab, set by
`oxplow.thread.mark`, for grouping threads the person's way; it changes
nothing else. A newly-seeded
stream ships with one thread titled `Thread` and no agent session; neither
does `oxplow.thread.create` open one. A person opens sessions
(`oxplow.agent_session.open`, whose default harness is the project's first
enabled agent, else the first declared (`HarnessRegistry::default`)).
The rolling `summary` field + `record_batch_summary` MCP tool were
removed in v13 — use the task log as the source of truth instead.

`closed_at` (migration v44, nullable TEXT) — set means the thread is
closed. A closed thread sits hidden from the rail; it surfaces only on the **Closed Threads**
page (the `closed-threads` page, the foundation's `closed-threads`
page over `v_thread`), which lists each closed thread with the work it
finished (counts by state) and a Reopen row action. `oxplow.thread.reopen` clears
`closed_at` and the thread returns to the rail as an open tab.
`ThreadStore.list()` already filters closed threads out, so every
existing surface (rail, work panel, dispatch) treats them as gone
without code changes; `listClosed(streamId)` is the dedicated reader
for the Closed Threads page.

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
when the parent stream's "Remove…" action archives every thread under
it (separate from `closed_at`, which is the user-visible "I'm done
with this thread" gesture). `ThreadStore::list_for_stream` filters
`archived_at IS NULL`, so archived threads vanish from every consumer
in lockstep with the stream that owns them.

`custom_prompt` (migration v18, nullable TEXT; read as `v_thread.prompt`, v5) — per-thread standing
instructions appended to the agent's system prompt after the stream-level
`custom_prompt`. Set via `setBatchPrompt(threadId, prompt)` on `BatchStore`;
IPC-exposed as `setBatchPrompt(streamId, threadId, prompt)`. Emits a
`thread.changed` event (kind: "prompt-changed") so the UI refreshes thread
state.

The thread's agent columns (`agent`, `acp_agent` (since dropped, V80), `resume_session_id`,
`pane_target`) moved onto `agent_session` in V32.

### `agent_session` — `SqliteAgentSessionStore` (`crates/oxplow-db/src/agent_session_store.rs`)

One agent slot a person opened on a thread (V32; `ses<n>`, ref
`agent_session:ses<n>`, model `v_agent_session`).

- **Columns:** `thread_id` (cascade), `kind` (`terminal` — a harness in
  a PTY; `action` — a one-shot session, not built
  yet), `harness` (a registry key: TEXT with no CHECK on purpose),
  `title` (empty until renamed), `resume_session_id` (the
  harness's own session id: `claude --resume`), `host`
  (reserved; NULL is the local machine), `opened_at`, `closed_at`,
  `closed_reason` (`closed`, `thread_closed`, `stream_archived`),
  `updated_at`, `pid` and `last_seen_at` (V49; V122 dropped
  `stopped_by_user_at`: a session ends by closing its tab and comes back
  from Earlier sessions, with no stopped-but-kept state),
  `native_title` (V78: the name its harness gives the bound conversation —
  Claude's `/rename` or generated title, through a prompt hook's
  `session_title` — cleared when the slot moves to another, never
  copied across) and `fork_next` (V78: the next launch forks the bound
  conversation instead of resuming it; `fork_conversation` sets it on a
  NEW session it opens, never on the source). `v_agent_session.display_name`
  is `title`, else `native_title`, else harness and opened date: the one
  place a session is named. Rename sets `title` (the alias; clearing it
  falls back to `native_title`); naming the conversation in the harness is
  a separate call where the harness has `native_rename`.
- **Live identity:** `pid` is the OS pid of the process the daemon
  spawned for the session (`HookIngestService::record_agent_pid`, called
  at every spawn, so a restart overwrites it; also stamps `last_seen_at`).
  `last_seen_at` is the newest hook from it, written at most every 30 s
  (`touch_seen_tx`, in `session_row_tx`). A pid alone can be dead or
  reused: a reader pairs it with a recent `last_seen_at` or checks the
  process exists (`daemon_supervisor::process_alive`). Agent-neutral —
  no harness-private socket paths.
- **Lifecycle: the row is the slot, not the process.** `closed_at` is set
  only by closing the session (`oxplow.agent_session.close`), its thread
  (`thread_closed`) or its stream's archive (`stream_archived`) —
  `agent_stores::close_session_tx`, which also ends its open turns and logs
  `stopped`; the caller stops its process after commit. A
  harness process ending (a PTY exit, `/clear`) logs
  `agent.session.ended` and leaves the row open, so its tab keeps the
  ended notice and the resume id.
- **Resume id and its bindings (V78):** `resume_session_id` is the open
  row of `agent_session_binding` (`v_agent_session_binding`: `native_id`,
  `origin`, `bound_at`, `unbound_at`); `bind_tx` is its one writer — it
  ends the open binding, opens the next, clears `native_title` and
  `fork_next`. The hook ingest binds the id a prompt or `SessionStart`
  names when it differs: `started` (the slot had none), `forked` (a fork
  launch's new id) or `switched` (a native `/resume`, `/branch`).
  `chosen` is bound by `agent_session.open { resume }` and by `fork_conversation` (the conversation the person picked); there is no move of an existing session onto another conversation. Rows an earlier `resume_conversation` wrote keep `chosen`. A `/clear` of
  exactly the bound session and `start_fresh` unbind (`unbind_tx`,
  conditional on the expected id, so a newer one stays); a launch never
  does. `bound_elsewhere_tx` keeps one conversation in one open slot per
  host. V78 backfilled a `started` binding for every bound slot.
- **Turns and events carry it.** `agent_turn.agent_session_id` (nullable:
  a hook from an agent oxplow didn't start claims none) and the
  `event_log.agent_session_id` anchor (`Anchors.agent_session_id`, named
  apart from `agent_turn.session_id`, the harness's string). V32
  backfilled one session per existing thread (a `terminal`; closed with the thread, or `stream_archived` with its
  archive) and anchored its turns and `agent.*` events to it.
- **Migration:** V32 drops the thread columns with `DROP COLUMN` (it ran
  when migrations still had foreign keys on, where rebuilding `threads`
  cascade-deleted its children); `migrate_and_compile` drops the model views first, so no
  `v_thread` blocks the drop. `v32_moves_agent_columns_onto_one_session_per_thread`
  checks rows, anchors, the dropped columns and the children.
- **V80 (chat sessions removed):** open `chat` sessions were closed
  (`closed_reason='closed'`, their open turns ended with the answer
  'session closed'); every `chat` row became kind `terminal` as closed
  history; the table was rebuilt without `acp_agent` and with `kind IN
  ('terminal','action')`.
- **V53 (`kind` dropped):** every session is a harness in a terminal; the
  `action` kind was never built, so `kind` went (`v_agent_session` v12).

**Removed in v42:** the `auto_commit` column (added in v15) and the
`commit_point` / `wait_point` tables (added in v6/v7). Commits are now
user-driven only — the harness has no `git commit` path, no queueable
commit/wait markers, and nothing that steers the agent to commit. Consumers
running an older DB get the columns/tables dropped on first launch with
the new binary; existing rows are not migrated forward (no surface
reads them).

### Core's task tables (gone, `V34__drop_core_tasks.sql`)

Core keeps no tasks. A task is a row of the shared model `oxplow/task_record`, read as `v_oxplow_task` (with `v_oxplow_task_link`, `v_oxplow_task_comment` and `v_oxplow_request_link`) and written by its store (`.context/tasks.md`, `.context/models.md`). V34 dropped the tables core kept before — `task`, `task_link`, `task_comment`, `task_assignment`, `oxtask`, `oxtask_link`, `oxtask_note` and `request_task` — with their indexes, change-counter triggers and counter rows, and the `event_log_task_uri` index. What requests worked on is the model `request_task` (`model__oxplow__request_task`), which is another table.

### `symbol` + `symbol_capture` — code symbols (migration `V62__symbols_from_tree_sitter.sql`)

The definitions oxplow's own tree-sitter extraction finds in each stream's
files, restated per changed file at each snapshot by the `symbols.collect`
pump consumer, and one `symbol_capture` row per snapshot saying what the
collection covered. Read as `v_symbol` / `v_symbol_capture`; details in
[language-support.md](./language-support.md) "Symbols" and
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

### Thread notes (gone)

Thread notes (`thread_note`,
the per-thread capture pad of `oxplow.knowledge.add_note`) are gone: V112
dropped the table with its `page_ref` edges and search rows, and with the
commands, `v_thread_note`, `v_search_thread_note` and the `oxplow/thread_note`
resource. What an agent leaves for the next one is a request's record.

### Old UI units, answers and database dashboards (gone, V113)

A page is oxplow's one unit of UI (its file in an extension,
`.context/pages-and-tabs.md` "Page files"), so V113 dropped what the older
units kept: the old UI unit's catalog table and its listing view (pages are `page`,
`page_answer` and `page_param`, V110), `thread_answer` (what
the old show tool showed; an agent's shown page is an in-memory view now),
and `dashboard` / `dashboard_item` (a dashboard is a page whose view is a
grid of embeds). It deleted, without rewriting, every stored reference to
their URIs (the old page, answer and dashboard resources): bookmarks,
visits, remembered preferences, context items, page-ref edges and search
rows. Site search indexes pages instead (`v_search_page`, the launcher's
pages; pages with no category are left out).

### Handoffs (gone, V51)

Work handed from one thread to another (the `handoff` table,
`oxplow.handoff.*`) was removed on 2026-11-20: the context basket and tasks
do the useful part, and handoffs were used once. V51 dropped the table
with its `page_ref` edges, its `oxplow/handoff` search rows, its
`handoff.sent` and `handoff.settled` events and the `oxplow.handoff.*`
audit rows. `v_handoff` and `v_search_handoff` went with it, and so did
the `hnd` id kind.

### `terminal` (`terminal_store.rs`, V106)

A stream's terminals, one row per shell: `stream_id` (`ON DELETE CASCADE`;
an archive deletes the rows itself and ends the shells), `title`, `position`
(dense order within the stream) and `created_at`. Written only by the
`oxplow.terminal.*` commands through the store's `_tx` cores (`open_tx`
titles "Terminal N" — one past the highest numbered title, never below the
count — `rename_tx`, `delete_tx`); read as `v_terminal` (with its `uri`,
`oxplow:oxplow/terminal?id=trm<n>`, the key the shell's PTY is registered
under: [terminal.md](./terminal.md) "Terminals are rows").

### `request` and the work recorded on it (`V11`, `V16`, `V18`–`V25`)

A **turn** (`agent_turn`) is one prompt-to-stop run of an agent. A
**request** is the turn that started something and every turn that
followed from it (a background job's report, a subagent's hand-back, a
matched peer message), joined by the cause links set when each turn opens.
Efforts were removed (2026-11-10); the request and its evidence replace
them. **V25** dropped their data: the `effort`, `effort_file`,
`effort_metric_delta`, `effort_observation_row` and
`effort_evidence_state` tables; every `effort_id` column
(`agent_nudge`, `agent_token_usage`, `agent_tool_call`, `metric_capture`,
`once_mark`, `snapshot_op`, `event_log` and the old run table, now replaced
by `contribution_run`; the tables with a foreign key rebuilt, every token
row and capture kept); the effort
events, their measures, specs and the effort producers' captures; the
`effort` change kind; the effort-projected `page_ref` edges and search
entries; the `effort_policy` capability rows; and an effort-only once
mark. A take an effort asked for (`effort_start` / `effort_end`) stays,
as `manual`, in `snapshot_op` and its `snapshot.taken` payload. A nudge
whose cause only an effort told apart keeps its row without its cause.
`job_run_context` drops `anchors.effort_id` from stored contexts.
On a copy of a real database (2.1 GB, 150k captures, 1.9M facts, 340k
events) it ran in 12 s.

- **`request`** (V11): `id`, `thread_id` (`ON DELETE CASCADE`),
  `first_turn_id`. `agent_turn.request_id` is the request a turn belongs
  to: its cause turn's, else a new one. The row holds identity and
  membership only. Work recorded against a turn (tool calls, tokens, test
  runs, jobs) is the request's through the turn, so no other row repeats
  it. `v_request` reads the totals through the turns and the state live:
  `waiting_on_person`, `running`, `waiting_on_work` (a job one of its
  turns started has not reported) or `finished`. Its URI is
  `oxplow:oxplow/request?id=req<n>`.
- **`request_task`** (V16): the tasks a request worked on, one row per
  (request, task URI, basis). `basis` is `prompt` (a person's prompt in
  the request named the task), `command` (the agent ran a task command on
  it in a turn), `transition` (a task transition in a turn) or `assigned`
  (it was in progress and assigned to the thread when a turn of the
  request ended; the weakest). `turn_id` is the turn the evidence came
  from. It is observed, never
  stated: nothing here opens, closes or moves a task. `task_uri` is not a
  foreign key, because a task's store may drop it. Bundled's model
  `request_task` holds these rows and `v_oxplow_request_link` reads them. A task's requests are the task's work; the task page reads
  them there.
- **`commit_request`** (V20): the requests a commit's changes came from,
  one row per (commit, path, request). `basis` is `named` (an edit call of
  one of its turns named the path), `reported` (the harness said a shell
  call changed it) or `observed` (it changed while the request ran and
  nothing else says). `confidence` is `sole` (one request changed it since
  the previous commit that held it) or `shared`. `v_commit_request` reads
  it; the task→commit edge derives through requests
  ([git-integration.md](./git-integration.md) "Linked to the requests that
  made it"). `file_snapshot.git_oid` (V20) is a captured file's git blob
  id, so a commit's content matches a capture without reading bytes.
- **`agent_tool_call_path`** (V18): every file a tool call names
  (`named`) or its harness reports it changed (`reported`; Claude Code's
  `bashEditDiff`). `agent_tool_call.path` keeps the first only. V18
  backfilled the rows from the finishes' payloads. `v_tool_call_path`
  reads it; `v_tool_call` carries `agent_session_id`, `subagent_id` and
  `subagent_kind`.
- **`turn_file`** (V21): the files that changed during each turn: turn,
  path, stream, first and last change, `timed`, `shared`. It is a time
  join of a turn with its stream's snapshot line, stored. The
  `snapshot_op_turn_file` trigger writes it when a take is logged (a take
  that changed nothing logs none); V21 backfilled it once. Several writers
  can change one worktree at once, so it is best-effort. `v_turn_file`
  adds the claim (`named` or `reported`, from `agent_tool_call_path`) and
  the confidence: `claimed` (a call of the turn named or reported the
  file), `shared` (another turn on the stream ran while it changed) or
  `sole`.
- **`request_analysis`**, **`request_metric_delta`**,
  **`request_observation`** (V22): a request's stored analysis on one
  stream (a request can span streams). `request_analysis` holds the span
  of the request's turns there, its start and end snapshots, and `sig`,
  the signature of the inputs it was computed from (a differing one is
  stale). The deltas and the observations (tests, diff coverage, static
  analysis) hang off it with `ON DELETE CASCADE`. Read as
  `v_request_metric_delta` and `v_request_observation`; see
  [collection.md](./collection.md) "A request's analysis".
- **Pacing** (V24): `once_mark.request_id` and `agent_nudge.request_id`
  scope a one-shot mark and a nudge's dedupe to the request of the turn
  they belong to, so a once-per-request hint fires again for the next
  request.
- **A run** (a test, coverage or analysis capture) belongs to the turn
  whose tool call caused it: `metric_capture.turn_id`, read through
  `agent_turn.request_id`. `v_test_run` carries it. See
  [metrics.md](./metrics.md) for how request reads use it.

A request has no task column. A task links to requests through
the model `request_task`; the page reading a task's requests joins
`v_oxplow_request_link` to `v_request`.

### `snapshot` + `file_snapshot` — `SnapshotStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Time-ordered snapshots in two tables (the actual schema; an earlier
draft of this doc described a single `snapshot_entry` table with a
`version_hash`/`source` manifest — that pre-V13 shape is gone):

- **`git_branch`** (V86 adds `upstream`, `ahead`, `behind`) and
  **`stream_head`** (V86: `stream_id`, `head_sha`) — restated whole by the
  refs scan (`commit_indexer::refresh_refs`) and read as `v_branch` (v2) and
  `v_stream.head_sha` (v4): a local branch's tracked branch and how far they
  have parted, and where each stream's workspace is (detached or not).
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

**Content identity vs storage address (V96).** `blob_hash` is
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
and knowledge page drift. `stats_for_snapshot` / `list_changes_for_snapshot`
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
commands (`oxplow.stream.create`,
`oxplow.stream.archive`) register/unregister at
runtime. Callers resolve the right service via
`snapshot_captures.get(&stream_id)` (when the stream is known)
or `snapshot_captures.primary()` (for project-shared surfaces
like the oxwiki file watch and freshness checks). A turn's takes
resolve via turn → thread → stream, so they capture against the turn's
actual worktree — a turn on a worktree stream never bleeds into the
primary's snapshot history.

`unregister` calls `SnapshotCaptureService::shutdown()`, which
fires a `tokio::sync::Notify` the `spawn_watcher` task selects on
alongside `rx.recv()`. This is required because the watcher task
holds its own clone of the service and its `FsWatcher` is a
task-local — dropping the registry's `Arc` alone never wakes the
task, so without the signal an archived stream's watcher would
linger until process exit.

**A turn's snapshots are found by time.** V25 dropped
`agent_turn.start_snapshot_id` and `snapshot_id`: the turn's end is its own
`turn_end` take (`snapshot_op.turn_id`), and the `turn` and `request`
change kinds find their two snapshots by time; its files are `turn_file`.
`agent_turn.task_id` is gone (nothing ever set it). **What opened a turn** (V2): `source` (`person`, `background_job`,
`subagent`, `peer`, `unknown`; NULL for a turn recorded before, until the
one-time conversion), `harness_turn_id`, `cause_turn_id` (FK to the turn it
goes on with), `cause_ref`, `cause_basis` (`reported`, `matched`) and
`sender`, set in the open transaction (`open_turn_tx` takes a
`TurnOpening`); `prompt` is the text that opened it, whatever its source.
V2 also rewrote the logged `agent.turn.started@2` rows to `@3`
(`source: unknown`). **How it ended** (V4): `outcome` (`completed`,
`interrupted`, `errored`, `session_ended`, `session_restarted`,
`session_closed`, `restart`; `unknown` for an old turn whose end left no trace), set in the
close transaction, with `error` (V9: the harness's words for an
`errored` end; V9 rewrote logged `agent.turn.ended@4` rows to `@5`); V4 gave ended turns the outcome their event logged
(an expired payload has none), told the ends logged `interrupted` or not
at all apart by the text oxplow had written into `answer`, cleared that
text, and rewrote `agent.turn.ended@3` rows to `@4` (an expired payload
only its version). The new enum-like columns of V2–V7 carry no CHECK:
Rust owns their values, and a CHECK could only grow by rebuilding the
table. **Requests** (V11): `request` (`thread_id`, `first_turn_id`) and `agent_turn.request_id`, set when the turn opens. **Messages between sessions** (V15, `agent_message`: the sending turn, recipient, text hash, the turn it was matched to on receipt, and V27's `notify_when_idle` ask with the `idle_notice_turn_id` of the notice that answered it). **A call's files** (V18, `agent_tool_call_path`: every file a call names, `named`, or its harness reports, `reported`; backfilled from the finishes' payloads), with an index on `file_snapshot(stream_id, captured_at)`. **A turn's files** (V21, `turn_file`: turn, path, stream, first/last change, `timed`, `shared`), written by the `snapshot_op_turn_file` trigger when a take is logged and backfilled once; `v_turn_file` adds the claim and confidence. **A request as a change** (V19 widened `change.kind`). **A commit's requests** (V20, `commit_request`: per (commit, path, request) with its basis and `sole`/`shared` confidence; `file_snapshot.git_oid` is a captured file's git blob id, so a commit's content is matched without reading bytes). **A request's analysis** (V22, `request_analysis` per (request, stream) with its span and input signature, `request_metric_delta`, `request_observation`; collection.md "A request's analysis"). **Pacing per request** (V24: `once_mark.request_id`, `agent_nudge.request_id`). **The question it ended on** (V10, `question`) is set in the close transaction. `agent_turn` open/close log
`agent.turn.started@3` / `agent.turn.ended@5` in the same transaction as
the row change; **background jobs** (V7, `agent_job`) are written beside
them by the hook ingest ([agent-model.md](./agent-model.md) "Background
jobs");
`event_log` has a `turn_id` index. A turn's `harness_turn_id` (the harness's own id; V13 indexes it with the thread) is set at the open or, for a harness that names the turn later, when it does.

**Turns record the model and reasoning effort they ran on (V82).** Nullable TEXT
`agent_turn.model` and `agent_turn.effort`, set by `close_turn_tx` from
`TurnEnd.model` / `TurnEnd.effort` when the turn ends (they ride the Stop's
`HookEnvelope.turn_run`) and published on `v_agent_turn`. A turn open, or
whose harness said nothing (opencode when the bridge can't read its last
message), has NULLs. The `AgentTurn` struct doesn't carry them: SQL reads
them.

**Plans an agent proposed are not recorded (V52).** V52 deleted the
`agent.plan.proposed` events; `v_agent_plan`, which read them, is gone.
See `agent-model.md` "Plans".
**Turns record how full the context was (V83).** Nullable INTEGER
`agent_turn.context_tokens` and `agent_turn.context_window`, set by
`close_turn_tx` from `TurnEnd.context_tokens` / `context_window` (the same
`HookEnvelope.turn_run`, so the same path as the model) and published on
`v_agent_turn` (version 6). `context_tokens` is the input of the turn's last
request, cache reads and writes included; `context_window` the model's
window. Claude Code and opencode give no window, so it is NULL there; a
turn open, or whose harness said nothing, has NULLs. A compaction is an
event, `agent.context.compacted@2`, not a column.

**Snapshots are the worktree's own record.** There is no turn, request or
`source` column on `snapshot` / `file_snapshot`. A turn's end is its
`turn_end` take (`snapshot_op.turn_id`); a request's analysis is bracketed by
`request_analysis.start_snapshot_id` and `end_snapshot_id`; a turn's files
by `turn_file`, a time join with the stream's snapshot line. Capture
de-dupes an unchanged tree to the latest existing snapshot id rather than
writing a near-identical row.

**Change detection.** `latest_stat_per_path(stream)` is per stream
(another worktree's rows are a different history). The startup sweep
short-circuits on `(size_bytes, mtime_ms)`: a file whose stat matches its
latest row is presumed unchanged and isn't re-read. When the stat moved
and the file is clean vs HEAD, an OID equal to the prior address is
unchanged; a prior row in the other space (an xxh3) is compared by
hashing the file, so committing a file already captured dirty records
nothing. Incremental capture skips a path whose bytes equal its latest row's
content — by `content_hash`, or for a git-backed row not hashed yet by
the bytes' git blob OID against its address — so a touch or a
formatter no-op is not a change, an oversize file whose
size and mtime are unchanged, and a tombstone for a path already
deleted. The sweep's reverse-deletion pass tombstones oversize files too
(it used to key on `blob_hash`, which oversize rows lack).

**Takes and the operation log (`snapshot_op`, V97, P2.2).** A
**take** (`SnapshotCaptureService::request_snapshot(TakeRequest)`) drains
the dirty set and hands the rows to `SqliteSnapshotStore::record_take`,
which writes — in **one transaction** — the new `snapshot` row (with
`branch`, `revision` when the tree is clean, and `tree_hash`), its
`file_snapshot` rows, one `snapshot_op` row and a `snapshot.taken@3`
event (anchors: stream, thread, turn, snapshot). It is the only
production write path; `create_snapshot` / `capture` / `capture_batch` /
`set_snapshot_revision` / `set_snapshot_branch` remain as fixture
seeders for tests.

- `snapshot_op(seq, stream_id, snapshot_id, parent_snapshot_id, trigger,
  thread_id, turn_id, at, elapsed_ms, budget_ms, over_budget,
  file_count)` (STRICT). `snapshot_id` is where the worktree is after
  the take; `parent_snapshot_id` where it was before (the stream's
  current snapshot = its latest op's `snapshot_id`). Readable as
  `v_snapshot_op`; listed newest first by `list_ops` (RPC
  `list_snapshot_ops { stream_id, limit? }`). V45 adds
  `provider TEXT` (the snapshot implementation that took it; NULL for
  older ops) and `contents INTEGER NOT NULL DEFAULT 1` (0 when the take
  kept no file bytes), exposed on `v_snapshot_op`. V46 adds `handle
  TEXT`: a snapshot provider process's own name for the state it marked
  (NULL for core's capture). The stream's newest op's handle is the
  parent core hands the provider's next mark (`.context/providers.md` "A
  snapshots provider").
- **Each file row carries when it changed** (V3): `file_snapshot.first_changed_at`
  / `last_changed_at`, the times the file watcher first and last reported
  the path changed since the previous take (a dirty-set entry keeps them,
  and a take that fails puts them back). NULL when the take found the
  change itself — the startup sweep, a file no watcher event named — which
  is then known only to lie between the two takes. They are the watcher's
  clock, so a join to turns by them is best-effort. Read on
  `v_snapshot_file` (v3).
- **A snapshot row carries its creating op** (P2.11): the stream listing
  (the `list_snapshots_for_stream` IPC op, `v_snapshot`) joins each snapshot's FIRST op for its
  `parent_snapshot_id`, `trigger` and `over_budget`. The diff view's
  "previous" for a single snapshot is that recorded parent
  (the `changes` page's `max(id) < N` on the stream), falling
  back to the next-smaller id only for a snapshot no op created; Local
  History shows the trigger and marks an over-budget take "slow".
- **An op is a take that captured files.** A request that found nothing
  waiting (or rows whose tree equals the parent's `tree_hash`, dropped
  in the same transaction) answers the stream's current snapshot and
  writes no snapshot row and no op: an empty take is not a fact about the
  worktree. When the request names a turn or thread (a turn's
  end) it still logs `snapshot.taken` with
  `unchanged = true` and `file_count = 0`, since `thread.checkpoint`
  waits on it, and the turn's `snapshot_id` still points at
  the current snapshot. A request nobody is anchored to (the quiet,
  startup and git-refs takes) logs nothing. A snapshot provider's mark
  keeps its op, changed or not, for the `handle` it names.
  `TakeOutcome.op_seq` is `None` for a request that logged no op.
- `trigger` ∈ `turn_end, quiet, startup,
  manual, git_refs, head_moved, run_measured` (`oxplow_domain::snapshot::
  SnapshotTrigger`). The ops backfilled for snapshots taken before the op
  log existed read `manual` (V71). `run_measured`
  (V161, `snapshot.taken@3`) is the take that pins a run's coverage and
  analysis to the code it measured (see [collection.md](./collection.md)).
- **HEAD moved on a clean tree** is its own op: `record_head_moved`
  re-stamps the snapshot the caller saw the clean tree at — refusing if a
  take has moved the stream on since, and the git-refs path
  holds the take lock from its drain through the stamp — with the new
  `revision` (and flips every exact-pin file ref on it) with a
  `head_moved` op, in one transaction.
- **A HEAD move is its own event**, `vcs.head.moved@3 { stream, from, to,
  snapshot? }`, logged by the git-refs listener once per move of this
  worktree's own HEAD, dirty tree or not (`snapshot` only when the tree
  was clean and the snapshot re-stamped). The listener reads its HEAD at
  start (starting is not a move), runs a `git_refs` take (draining
  anything dirty, then the re-stamp) and spends the move only once that
  take returned a snapshot and the event is logged: a take that fails, or
  finds no snapshot yet, leaves the move for the next ref event. Collectors
  that follow commits run on this event; `snapshot.taken` with
  `unchanged: true` is logged only for a request someone waits on.
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
against the request of the take's turn to populate `label` + `label_kind`
on each `FileSnapshot`. Snapshots no request made get `label: null` and
the UI falls back to a generic label.

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

**Retention: content expires, records don't.** Retention's job
is bounding the ON-DISK blob copies — the megabytes. The
`snapshot`/`file_snapshot` ROWS are **never deleted**: they are durable
records other subsystems replay — the per-path metric fold derives each
capture's restated set from them, and the ancestry anchors ride the
parent `snapshot` rows' git stamps — and they only weigh bytes. (The
earlier row prune rotted the fold's replay inputs out from under
durable captures; deleting rows to save disk was aiming at the wrong
mass.) The daily cleanup (`SnapshotCaptureService::run_cleanup`, boot +
every 24h) GCs `.oxplow/snapshots/objects/` down to
`retained_blob_hashes(cutoff)`: every row inside the retention window
(default 7 days, `snapshotRetentionDays` in `.oxplow/project.yaml`)
plus each `(stream, path)`'s newest row at ANY age — so every
worktree's current tree stays viewable/rollbackable forever. Older
content reads degrade to "expired": reading a snapshot's file (`read` of a file URI with `rev=snap:<id>`, the IPC read) → `None`
(`SnapshotFileError::Expired` in `oxplow_app::snapshot_files`), restores
refuse with an explicit message, never a half-restore.

**Content policy (hashes only).** `SnapshotCaptureService` has a
`ContentPolicy { Keep, HashesOnly }` (default `Keep`), read at each take
and set on every stream's service by `SnapshotCaptureRegistry::
set_content_policy` (a service registered later starts with it). Under
`HashesOnly` a changed file is read and hashed (xxh3) but no blob is
written, on the sweep and the capture path alike: its row is
`storage = oxplow`, `blob_hash = content_hash = hash`, an identity with
no bytes behind it — the state retention GC leaves an expired row in, so
no new storage class (and no rebuild of `file_snapshot`, whose `storage`
CHECK can't be widened). Git-clean, oversize and deleted rows are
unchanged. Diffs, change entries, tree hashes and "unchanged take returns
the parent" all work on identities, so they hold under either policy. A
take records `provider` and `contents` on its op (`TakeRequest.provider`,
`TakeRecord.contents = policy == Keep`). **`NoContents` vs `Expired`:** a
read whose blob is missing asks the op of the take that recorded the row
(`SqliteSnapshotStore::contentless_take_for_path` / `op_for_snapshot`):
`contents = 0` is `SnapshotFileError::NoContents { snapshot, provider }`
("never kept", naming who took it), anything else stays `Expired` ("kept,
then pruned"). The lookup runs only for a row whose bytes are missing, so
a normal read costs nothing extra. A switch deletes nothing: the record
holds both kinds of op, and a path unchanged since a kept take still reads
at a later hashes-only snapshot.

**An idle pipeline (a provider process active).** The capture registry
knows which snapshot implementations it serves (`set_serves`, filled by
`snapshots::register_built_ins`); `follow(active)` idles every service
while the active one is anything else. An idle service takes nothing of
its own: `request_snapshot` (the quiet, startup and baseline takes),
`request_snapshot_for_git_refs` and the startup sweep's staging answer
nothing, so no snapshot the provider never marked lands in its chain.
The watcher keeps the dirty set, and the first take after resuming
records what changed meanwhile. `request_take` (a built-in's mark) is not
gated: only the active implementation marks.
Only `oxplow`-class rows hold blob-store hashes; `git` rows reference
the git odb, which this GC never touches. The blob store is shared
across all streams, so GC runs at the project level and dedupes
identical content across branches.

Cleanup runs at runtime startup and again once every 24 hours via
`runtime.runSnapshotCleanup` (wired in `initialize()`, cleared in
`dispose()`).

**A worktree stream's history is sparse.** The primary's history holds its whole
tree, but a worktree stream holds only the files it touched. So in its sweeps and
rescans (`with_sparse_history`, set from the stream's kind), a file with no row that
is clean against HEAD records nothing. It has no history here, and nothing changed.
A row for each would be its whole tree "added" to a request's change. When git's stat can't vouch for it (every file of a fresh worktree was written in the
same second as its index), the bytes are compared with HEAD first. A file with a
row, an offline edit and a new file record as anywhere else.

**A take costs what changed.** It reads the latest row of only the paths it
looks at (`latest_stat_for_paths`), and a rescan of a directory only those under
it (`latest_stat_under`), each a seek on `idx_file_snapshot_stream_path`. Only a
stream's first sweep reads every path. `latest_stat_per_path` over all history
took 15.8 s at 5 million rows.

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

Toggling the list at runtime (the Directory page's Mark as Generated →
`oxplow.file.mark_generated`) takes effect immediately, no restart: the
command calls `SnapshotCaptureRegistry::set_generated`, which gives the new
lists to the filter of every live per-stream `SnapshotCaptureService` (each
has its own, rooted at its worktree) and keeps them for the services
registered later.

### `symbol` + `symbol_capture` — `SqliteSymbolStore` (`crates/oxplow-db/src/symbol_store.rs`, migration `V62__symbols_from_tree_sitter.sql`)

The symbol index: each stream's files' definitions, restated per changed
file by the symbol collector at each snapshot. `symbol` columns: `uri`
(`oxplow:oxplow/symbol?path=<path>&symbol=<name>`, no revision; unique per `(stream_id, uri)`),
`snapshot_id` (the snapshot the file was read at), `name`, `kind`,
`container`, `signature` (parameter types or dispatch value, NULL where the
language doesn't overload), `language`, 1-based `line`/`col` (the name) and
`start_*`/`end_*` (the extent). `symbol_capture` has one row per snapshot
handled: `files_collected` and `files_failed` (content unreadable at the
snapshot). V62 cleared both tables, added `signature`, made the ref unique
per stream and dropped `files_over_budget` / `files_without_server`; the
boot backfill (`symbol_collector::backfill`) refills an empty index from each
live stream's latest snapshot. Read as `v_symbol` / `v_symbol_capture`; see
[language-support.md](./language-support.md) "Symbols".

### Knowledge pages: no table

Core keeps no table of knowledge pages, their reads or their touches (V35
dropped `knowledge_page`, `knowledge_page_read` and `knowledge_page_touch`).
A page is a row of the shared model `oxplow/knowledge_record`
(`model__oxplow__knowledge_record`), kept by a store; the reads and the
touches are bundled's own models; the datasets are `v_oxplow_knowledge_*`
([knowledge.md](./knowledge.md), [models.md](./models.md)).

### `page_ref` — unified cross-page reference graph (`crates/oxplow-db/src/page_ref_store.rs`, migration `V11__page_ref.sql`)

One row per directed edge `(source) --ref_type--> (target)` across
every page kind. Replaces the in-memory `computeBacklinks` indexer
that used to live in the desktop frontend; both backlinks ("what
points at me?") and outbound ("what do I point at?") are SQL
queries against this table.

Columns (V99): `source_uri, target_uri, source_resource,
target_resource, ref_type, source_extra, local_snapshot_id,
closest_vcs_rev, vcs_rev_exact` (PK on `(source_uri, target_uri,
ref_type)`). Each end is one URI, an identity (no `#view`; a file edge's
version pin rides in the columns, not the URI). Indexes on `target_uri`
for backlinks, `source_uri` for outbound, `target_resource` /
`source_resource`, and `local_snapshot_id` for the cascade-on-commit-attach
update. The resource is denormalised next to each URI so
resource-filtered queries don't need to parse the URI.

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
write cascades: `page_ref` rows
pointing at that snapshot get their `closest_vcs_rev` set and
`vcs_rev_exact` flipped to 1. The capture-time resolver lives
in `oxplow_app::file_ref_version`; callers don't pass any of these
fields by hand — the oxwiki file sync fills them automatically.

**A model row's body keeps its pins.** A shared model's markdown column with
`pins: true` is projected by `model_store` (`models.md` "Markdown bodies"): the
row's file and directory refs are `file_pin` / `dir_pin` edges, each pinned to
the primary stream's latest snapshot when it is first added, and kept as they
are when the body is rewritten (editing unrelated prose doesn't re-stamp every
ref). `oxplow.refs.verify { uri, files? }` re-pins the named refs (all when
none are named); `v_ref_pin` says which are stale. A directory ref's verified
file edges under it survive a rewrite while the directory is still cited.

Every stored `source_uri` / `target_uri` is a canonical URI (see
`.context/resources.md`), built by the owner's builder
(`uri::build`) and never concatenated:
- `oxplow/knowledge` — `?id=<id>&store=<store>`
- `oxplow/task` — `?id=<id>&provider=<provider>` (`tsk42` / `oxplow`, `ENG-12` / `issues`)
- `oxplow/file` — `?path=<repo-relative path>`
- `oxplow/dir` — `?path=<repo-relative path>`, no trailing slash
- `oxplow/commit` — `?sha=<full sha>`
- `oxplow/finding` — `?id=<rowid>`

V99 rewrote the stored rows from the old `(kind, id)` pairs; the boot
backfill regenerates the derived ones. `V100__bare_ids.sql` then moved the
event payload fields named for one resource (and undo inverses, pending
proposals) from URIs to bare ids, at new event versions; see
[resources.md](./resources.md) "The migrations (V99, V100)".
`V103__resource_page_drop.sql` dropped `resource.page` (a resource opens
as itself; the pages that render it declare `for`), and `v_resource` is
at v4 without that column.
`V101__uri_names.sql` then renamed the columns that held a URI under the
name `ref` (`task.uri` / `parent_uri`, `task_link.from_uri` /
`to_uri`, `search_entry.uri`, …) and moved
a provider's page event to v2 (since removed); see
"The rename (V101)" there.

**Writers own slices by `ref_type`.** A single `source_uri`
can have rows from multiple owners — a task's body,
link and comment edges (restated from the task model) all land under
the item's one `source_uri` but with distinct `ref_type`s.
`SqlitePageRefStore::replace_source_for_ref_types` lets each
writer wipe + re-insert only the rows whose `ref_type` it owns,
so other owners' rows survive.

Writers (one per source kind / slice):

The vocabulary speaks of tasks, whichever list they're on (V21
renamed the `task_…` spellings; V36 made a mention of any model's record one
type: `record_mention` / `comment_record_mention`; a link is `task_link:<type>`).

| Owner | Source | Slice (`ref_type`s) |
|---|---|---|
| `model_store::write_tx` (`oxplow-db`), for a model's markdown columns | the row's URI | the body's edges: `file_pin` / `dir_pin` (pinned columns) or `file_mention` / `dir_mention`, `record_mention`, … |
| (the same) | comment slice: the comments' mentions, as the item's own edges (V28 dropped the old `oxtask_note` sources) | `comment_file_ref`, `comment_dir_ref`, `comment_record_mention`, `comment_finding_mention`, `comment_commit_mention` |
| `analytics_stores::SqliteCodeQualityStore::append_finding` | the finding's URI | full source |
| `commit_indexer.rs` (`oxplow-app`) | the commit's URI | full source — diff yields `touched_file`, message yields the same body-mention set |

The shared extractor `oxplow_domain::uri::extract(body) ->
ExtractedRefs` is the single parser used by every writer that
takes a free-text body, so knowledge/task/commit-message ref
recognition stays in lock-step. Pure projections live in
`crates/oxplow-db/src/page_ref_projections.rs`.

Reader: `SqlitePageRefStore::list_backlinks(target, limit)` and
`list_outbound(source, limit)` (URIs). Both
are exposed as Tauri commands (`list_backlinks` / `list_outbound`,
which decorate each row with a best-effort `source_label` from
the source store). An agent reads the same edges as the `v_page_ref` model
(`source_uri`, `target_uri`, `ref_type`) with `oxplow_query_sql`; there is no MCP tool.

Boot-time restate: `oxplow_app::page_ref_backfill::run(...)` re-
projects every task (`task_refs::all_refs_tx`, deleted ones
included — restating one clears it), finding and thread note into the table,
idempotently — the graph's repair path after a migration that resets
`page_ref` (V92) or a writer's drift. It runs at boot only when the
schema version or the projection code changed since its last run — a
hash of the files that decide a source's edges (`PROJECTION_SOURCES`:
the backfill, `page_ref_projections.rs`, `task_refs.rs`), not the build, which every dev rebuild changed
(`needs_repair`; recorded as `asset_state` row `page_ref_repair`):
between those the writers keep the graph current. A new projection file
belongs in that list. The hash is of the text, comments included, so a
comment edit runs it too; that is cheap, because a run writes only the
sources whose stored edges differ from their projection
(`replace_sources` compares first): a run over a current graph is reads,
with no write and no change event. Knowledge page bodies and recent commits are covered by their
own initial-scan paths and don't need separate backfill. Efforts were
removed (2026-11-10), and their slice of the graph (touched files, summary
refs, declared impacts) went with them.

### `event_log` + `event_consumer_checkpoint` + `event_dead_letter` + `command_run` — `SqliteEventLogStore` (`crates/oxplow-db/src/event_log_store.rs`, migration `V93__event_log.sql`)

The event log, built in P1.4. **State tables hold current truth; the log
records activity and state changes**, and it is written in the *same
transaction* as the change — the outbox pattern — so the two never
disagree. The envelope is `oxplow_domain::events::Envelope`
(`crates/oxplow-domain/src/events/mod.rs`): `id` (UUIDv7 text, sorts by
time), `type` (`namespace.name`, snake_case, validated on append), `v`
(schema version), `at`, `source` (`agent:thr3`, `human`, a page's URI (`oxplow:oxplow/page?page=<id>`, from
`uri::build::page_uri`), `system`, or `system:<component>` from `uri::build::system_source` —
`system:task_service`, `system:hook_ingest`, `system:snapshot_capture`),
`anchors` (nullable stream / thread / turn / snapshot / agent
session columns, so per-anchor timelines are indexed range scans; an
event that names a thread also carries its stream, and agent activity
carries the agent session it came from — `hook_ingest`'s resolution,
agent-model.md; V153 adds a partial index on an
event's item ref, for a provider read's "did this
change?" lookup — providers.md), `subject` (JSON array of canonical
URIs built with `oxplow_domain::uri::build` and validated against the
resource registry on append, see [resources.md](./resources.md)), `payload` (JSON;
validated against `type@v`'s schema on append), `payload_hash` (reserved for
forgettable bodies stored by content hash), `cause`, `dedupe_key`
(UNIQUE — the emitter derives it from the occurrence, so an at-least-once
producer's second `append_tx` fails with `Constraint` and writes nothing;
`append_unique_tx` instead inserts with `ON CONFLICT (dedupe_key) DO
NOTHING` and reports whether it landed, so a concurrent twin is skipped by
the insert itself rather than a check before it).
`seq` (AUTOINCREMENT) is the delivery order; `id` is the public identity.
V73 initializes the immediate post-tool advisory consumer from the existing
post-tool advisory checkpoint. On upgrade, the new consumer starts at the
same position in the log and can process the outstanding events.
The existing consumer remains the completion barrier for both groups of
post-tool advisories for extensions that declare it in `after:`.

These are the schema's first **STRICT** tables; every later spine table
is STRICT too.

**Schemas (P1.5).** `oxplow_domain::events::schema` has one
Rust type per `type@v` (`trait EventType { TYPE, V, Payload: JsonSchema,
upcast }`) and `EventSchemaRegistry`, which holds every type the log
accepts. It sits with the resources in the running `Vocabulary { events,
resources }`, held in the swappable `VocabularyHandle` `Services.vocabulary`
(P8.D1; see resources.md) — `Vocabulary::core()` is every core type; the golden files under `schemas/events/` are the
authoritative list (the `core_registry_knows_every_core_type_and_version`
test pins it). An extension's types join it as data, not Rust types:
`register_declared(extension, DeclaredEventType { event_type, v, schema,
summary, upcast })` (P8.D2) refuses a core namespace (`CORE_NAMESPACES`,
§5.3), any namespace but the extension's own (`extension_namespace`: its
name with `-` read as `_`), a schema that doesn't compile, a duplicate
`type@v`, v0, and a version past 1 without an upcast. The upcast is the
extension's Starlark (`extension_event_types::starlark_upcast`:
`transform({from_v, payload})`, sandboxed with the command-script budget,
no host); its output is validated against the newest schema. What the
running vocabulary registered is restated, per `type@v`, into
`event_type_contract` (V144; `v_event_type`), which keeps a declared
type's first schema as its contract and a removed extension's types as
`registered = 0` (P8.D3, extensions-events.md "Event types"). `append_tx`
refuses an unregistered `type@v` or a payload that fails its schema
(`DomainError::Invalid`, naming the JSON path) before writing. Core
producers build envelopes with `Envelope::typed::<T>(source, &payload)`
so the shape is checked by the compiler too. **A payload owns its enums**:
the golden schema is a published contract, so an event uses its
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
of the old file. **Core keeps only each type's newest version:** when core
bumps one, a migration rewrites the logged rows to it in the same change
(V31, V70), and the old version's type, upcast, golden and fixture go.
An extension's declared types keep their older versions and upcasts,
since their rows can't be rewritten by core. `OXPLOW_BLESS=1` writes a golden for a type that has
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
it to, and the row was valid when it was appended. A published version's
Rust doc comment is part of its schema — don't edit it.

**`event_content` (V102, P3.2)** holds large or sensitive event bodies —
tool input and output, prompts — by the xxh3-128 hex of their bytes (the
blob store's hash, so refs are uniform): `hash PK, namespace, bytes BLOB,
size, created_at`, STRICT. A payload carries `ContentRef { hash, size,
truncated }`. A JSON body is stored by `put_json_tx`: serialized
canonically (object keys sorted at every depth, so equal bodies share a
hash whatever order their keys arrived in) and cut to 256 KiB on a
character boundary — `size` is the whole body's, `truncated` says only the
first part was kept. It is read through `oxplow_app::event_bodies::read`,
by **event id and body** (`input` / `output`), never by bare hash: the
person's IPC read sees any stream, the MCP `oxplow_read` of `event:<id>#…` only the
caller's own stream's events, and both return at most 64 KiB of text with
`truncated`. The retention
sweep deletes a body after its namespace's window and the event stays. It is separate from the snapshot blob store because the
retention differs (and the blob store's GC only knows snapshot rows).
`v_event_content` exposes everything but the bytes. `event_log` gained
`payload_expired_at`: `payload` is NOT NULL, so payload expiry writes `'{}'`
and stamps it. The sweep is `oxplow_db::event_retention::sweep` (P3.11).
It runs ten minutes after boot and then daily. The same sweep applies the
run retention rules described below.
The event payload sweep works in batches of 5,000 rows per transaction
through the partial index `event_log_live_payload (type, at) WHERE
payload_expired_at IS NULL`, V103, read as a `type` range because a `LIKE`
can't use a BINARY index; windows in `oxplow_domain::events::retention`'s
`CORE_WINDOWS`: `agent` 30 d payload / 14 d body,
`test`, `code`, `collector` and `effect` 90 / 30, `ui` (op errors the
person saw and their seen / dismissed marks) 30 / 14, `file` (`file.saved`, each editor save,
`oxplow.file.save`) 30 / 14, `ai` (no events: the request and response
of each of oxplow's own model calls, `ai_call.request_hash` /
`response_hash`, [ai-providers.md](./ai-providers.md)) 30 / 30, and every namespace core
doesn't own — an extension's, found by skipping through that index one
namespace per probe, and in `event_content` — `EXTENSION_DEFAULT` 30 / 14
(P7.B7), or the shorter window its extension declares
(`event_types.retention`, P8.D5: kept in `extension_event_retention`, V145,
keyed `(namespace, extension)` since V5 — extensions share a namespace, and
the sweep takes its longest window —
restated by the vocabulary reactor, longer than the default refused by
`check_declared` at load, and **kept when the extension is unloaded** —
its rows stay under the window it promised; an extension present without
a window drops back to the default — unless a declaring extension holds
the same namespace, which a namesake that declares nothing never speaks
for); core's state namespaces are kept). **A project sets its
own windows** with the person-only key `eventRetention: { <namespace>:
{ payloadDays, contentDays } }`: each sweep reads it, a core
namespace's replaces the default, an extension namespace's is capped at the
extension's window (`RetentionWindow::at_most`), and core state is refused
at load (`is_kept_whole`). A window is bounded (`window_problem`,): at least 7 days for a core namespace oxplow reads back (the
agent policy reads a turn's tool payloads), at least a day for an
extension's, at most `MAX_DAYS` (36,500 — beyond about 127,000 days the
cutoff arithmetic used to wrap into the future and wipe the namespace),
and its `contentDays` no longer than its `payloadDays`.
`Timestamp::from_unix_ms` computes in `i128`, and the sweep clamps a
cutoff to `MAX_DAYS` whatever reaches it. A window naming a namespace
nothing logs is kept — an extension may be installed later — and reported
(`SweepReport.unused`, a warning in the log). **An expired event is
history only**: `StoredEvent.payload_expired_at` carries the
stamp; the pump checkpoints past it without calling any consumer (a new or
renamed consumer replaying the log never sees `{}`); `retry_dead_letter`
refuses one; the sweep marks the pending dead letters of expired events
`discarded`; the status derivation ignores it. The activity log shows it
as "(details expired)" from the field, not from an empty payload.

**Projections of agent events (V102).** `agent_tool_call` gained `turn_id`
and `event_id` (`UNIQUE WHERE NOT NULL`). Since V6 a row is the
projection of one call: `tool_call_store::record_request_tx` opens it at
the `agent.tool.requested` (`request_event_id`, unique; `state` `running` or
`denied`), `record_finish_tx` closes it at the `agent.tool.finished` its
request caused (else writes one with no start), and `abandon_running_tx`
ends the turn's running calls at `agent.turn.ended`; a redelivered event
writes nothing new. V6 gave the rows already recorded their request (the
finish's cause), `started_at` and `duration_ms`. Every view over these (`v_tool_call`,
`v_token_usage`, `v_agent_nudge`) exposes
`turn_id`.

**The contract is `append_tx(&Connection, &Vocabulary, &Envelope)
-> seq`** (`Vocabulary::validate_envelope` checks the type, the payload
and every subject ref against the same snapshot), composed inside the producer's `Database::transaction` closure. The async
`SqliteEventLogStore::append` opens a transaction of its own and is for
activity with no state write (a tool call, a page view). Reads:
`read_after_tx(after_seq, limit)` (oldest first — the pump's cursor),
`get_tx(id)`.

**Delivery is at least once** — the `EventPump`
(`crates/oxplow-app/src/event_pump.rs`, P1.7). A consumer is
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
producers call after their commit — or every 5s). A handler error that is retryable (`DomainError::
Busy`) is not a poison event: the delivery transaction fails and retries,
and if the database stays busy the event waits, checkpoint unmoved
(review).

**The consumers (after P3).** Every reaction to the log, by checkpoint
name. "Boot" consumers are registered by `Services::boot` and run wherever
`Services` does (tests included); "boot.rs" ones need an `Arc<Services>`
and are registered by `crate::boot` — a test that wants them calls their
`register` itself.

| Consumer | Kind | On | Does | Registered |
|---|---|---|---|---|
| `tool_call.project` | sync | `agent.tool.requested`, `agent.tool.finished`, `agent.turn.ended` | the `agent_tool_call` row (one per call: opened at the request, closed at the finish, abandoned at its turn's end; V6) | boot |
| `ui.push` | sync | `snapshot.taken` (that recorded files), `vcs.head.moved` | the renderer's `SnapshotTaken` on the UI bus (P7.B6) | boot |
| `metrics.entity_states` | async | `snapshot.taken`, `collector.synced` | re-captures state entity metrics, throttled per metric (P7.B6) | boot |
| `config.workspace_filter` | async | `config.changed` (`generated`) | the snapshot captures' filter | boot |
| `thread.checkpoint` | async | `snapshot.taken` (`turn_end`) | logs `thread.checkpoint` (changed since the turn began, writing tool count) | `Services::new` |
| `collection` | async | `agent.tool.finished` (Bash) | test / analysis / coverage captures, `test.*` events, nudges | boot |
| `advisories.post_tool` | async | `agent.tool.finished` | post-tool advisories that follow collection, persisted as nudges | boot |
| `advisories.post_tool.immediate` | async | `agent.tool.finished` | post-tool advisories that need only the tool projection, persisted as nudges | boot |
| `advisories.turn_end` | async | `thread.checkpoint` | turn-end advisories (hints) for the request holding the turn's end, persisted as undelivered nudges for the next prompt | `Services::new` |
| `token_usage.turns` | async | `agent.turn.ended` | a turn's token rows (transcript tail or reported counts) | boot |
| `search.index` | async | `snapshot.taken` | the search index's file contents (the other kinds are assets, `kind_search.rs`) | boot.rs |
| `config.extensions` / `config.providers` / `config.metrics` | async | `config.changed` (`extensions`; `extensionInstances`, `activeProviders`; any key) | after the in-memory swap: the extension catalog's change signal; the provider registry reconciles; the metric catalog reseeds (P7.B6) | boot.rs |
| `extension_datasets.entities` | async | `collector.synced` | the extension models compile again (a new entity may let one) (P7.B6) | boot.rs |
| `change.analyze` | async | `snapshot.taken` (that recorded files), `vcs.head.moved` | re-analyzes the stream's working change and the requests' changes, skipping an event a newer one supersedes; dead-letters a failure naming the stream (P7.B4) | boot.rs |
| `collector.triggers` | async | what enabled collectors' `on:` name (never `collector.synced`) | runs each matching collector once per event (`collector_checkpoint.last_event_id`). Entity collectors write rows and fact collectors use the fact engine. On `snapshot.taken`, delta collectors run when the snapshot recorded files and whole-tree collectors run on every take. Other events use the stream's latest snapshot. The transaction also records the run and logs `collector.synced@2`. | boot.rs |

**Async consumers (P2.6.2).** `trait AsyncEventConsumer { name,
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
what they consume). **Ordering:** an async consumer may declare
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
effect.
**Sync projections come first.** Whatever its `after()`, an async
consumer (a collector, an effect) handles an event only after every sync
consumer has handled it and all earlier events: `run_slot` runs the sync
consumers up to date before each batch it delivers (`run_sync`). A
collector's input may therefore read any projection (`v_tool_call_path`,
`v_agent_turn`) of its trigger event without an `after:`. Between async
consumers there is still no order but `after`; events reach one consumer
in log order, so a collector sees what an earlier event's collector wrote.

Letters are `pending | retried | discarded`; the
person's moves are `oxplow.delivery.retry { id }` (re-runs the consumer now;
`retried` on success, else `pending` with the new error; refused unless
the letter is `pending`) and `oxplow.delivery.discard { id }` — commands
of a person or a page for one, never an agent's (`commands/delivery.rs`);
the list is `v_event_dead_letter`. Also
`v_event`, `v_event_dead_letter`, `v_event_checkpoint` in the semantic
layer (V94). Nothing is skipped silently.

`command_run` is the command detail table named by V68. Its unique,
required `run_id` names a `contribution_run`. It keeps the integer
`id`, the command's name, `input_json`, `result_json`, `event_id`,
`inverse_json` and `undone_by`. Reads and no-ops leave the payload and
inverse columns NULL. The bus writes the run and its details through
`command_run_store::insert_tx`, then links an event or undo when there
is one. A nested write's details land with its transaction. A rolled-back
child keeps its outcome and its own error without payloads or an inverse.
Successful writes cancelled by the parent transaction carry the rollback reason.
`SqliteCommandRunStore` joins the run for the common fields, and
`v_command_run` publishes them. V64 preserves existing audit IDs and the
old AUTOINCREMENT high-water mark before backfilling detail rows.
See [commands.md](./commands.md).

**Event cores.** A store core that logs an event in its own transaction
(`open_turn_tx` and `close_turn_tx` log `agent.turn.started@3` and
`agent.turn.ended@5`) takes an `EventCtx { schemas, source, cause
}` (`event_log_store`): `EventCtx::system(schemas, "task_service")` for a
system writer; a bus command will pass its actor's source and its
`command.executed` id as cause. `anchors_for_thread_tx` gives the thread
+ its stream for any event about work on a thread.

V93 also dropped `task_event` (a per-task audit table nothing had written
since V1) and wiped `page_visit` (its rows carried pre-canonical tab ids).

### `usage_event` — `UsageStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Generic (kind, key) usage tracking. Append-only event log with columns
`stream_id (nullable), thread_id (nullable), kind, key, event,
occurred_at`. Aggregates (most-recent, most-frequent, currently-open)
are derived by query rather than stored, so adding a new "kind"
(editor file, task, future surfaces) needs no schema change.
Indexes: `(kind, key, occurred_at DESC)`, `(stream_id, kind,
occurred_at DESC)`, `(thread_id, kind, occurred_at DESC)`. Both scopes
are recorded simultaneously — `stream_id` is the workspace tab,
`thread_id` is the selected thread within it — so consumers can roll
up by either dimension or intersect them.

The store coalesces rapid repeats: if the most recent matching
`(kind, key, event, stream_id)` row is younger than `coalesceMs`
(default 30s), `record()` bumps its `occurred_at` instead of inserting
a new row. This keeps history clean when a user re-selects the same
target several times in quick succession.

Current write hookpoints (all in `apps/desktop/src/App.tsx`, all pass both
`streamId` and `threadId`):

- `wiki-note` — `handleOpenNote` records a visit when a note becomes
  the active center tab. Nothing reads it any more: the Knowledge page's
  "Recently visited" is the `wiki-visited` page over `v_page_visit`.
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

### Per-run observations — **RETIRED** (dropped in migration `V39__drop_effort_observation.sql`)

This table and its store are **gone**.
Coverage / test / static-analysis facts live in the **fact substrate**
(`fact` rows under `metric_capture`, see `.context/metrics.md`); the rich
detail that used to live in `payload_json` (test suite/case tree, coverage
per-file line-sets, analysis payload) rides verbatim in
`metric_capture.detail_json` (the `{"kind": …, "payload": …}` envelope, T-E1).
A request's rows are `request_observation` (V22, read as
`v_request_observation`), computed from the request's turns' captures
(`metric_capture.turn_id`) and stored with the signature of their inputs in
`request_analysis.sig`, so a request whose runs or files move is
recomputed; the panel and an agent (`oxplow_query_sql`) read those. The `provenance`/`source` trust spine and the `observed`/`asserted`
distinction carry on every capture (see `.context/metrics.md`).

### `metric_definition`/`metric_run`/`metric_sample`/`metric_finding` (+ `metric_dimension`/`metric_subject`) — **RETIRED** (dropped in `V49__drop_legacy_metric_tables.sql`, T-E3/)

The V38 metric cluster + `metric_store.rs` are **gone**. The fact substrate
(`measure`/`dimension`/`metric_spec`/`metric_capture`/`fact`, V43+) is the sole
metric store: producers write facts on captures, metrics are SPECS aggregated
at read time by `MetricEngine`, and the run identity for attribution is the
capture id. See `.context/metrics.md`. A capture names its stream, thread
and — V157 — the agent turn it was measured
in (`turn_id`, FK `agent_turn`, SET NULL; NULL when no turn produced it).

> **Timestamps are fixed-width by construction (→).**
> Timestamps are RFC 3339 TEXT that SQLite compares lexicographically. The
> `time` crate's default formatter trims trailing fractional zeros, and `'Z'`
> sorts after every digit, so a trimmed `…20.5Z` sorted *after* `…20.500001Z`
> and a whole-second `…20Z` after everything in its second — inverting
> `ORDER BY …_at` and window comparisons. The first fix (`canonical_ts`, used
> by six stores) left the other twelve trimmed; the flaky
> `thread_grows_and_orders_oldest_first` was two thread messages inside one
> trimmed prefix. Now `Timestamp` itself serializes to the fixed-width
> `YYYY-MM-DDTHH:MM:SS.ffffffZ` (27 chars; `Timestamp::to_text`,
> `oxplow-domain/src/time.rs`), every store writes through the one
> `database::ts_to_string` / `string_to_ts` pair, and **V95** normalized every
> `*_at` / `at` TEXT column that existed at V94 (a test in `database.rs` pins
> that the migration names every such column). Parsing still accepts any RFC
> 3339 text. A new store needs no special handling — there is no other way to
> turn a `Timestamp` into text.

### `metric_cube` + `metric_live_fact` + `metric_cube_state` — the aggregate cube (`crates/oxplow-db/src/fact_store.rs`, migrations `V62__metric_cube.sql` + `V63__branch_aware_cube.sql`)

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
  /V65 — see `.context/metrics.md`). `branch` is `''` for a branch-less
  capture (a WITHOUT ROWID PK can't hold NULL; the `''` mapping lives in the
  store layer only). Sized by live subjects, not history.
- **`metric_cube_state`** — the watermark, per `(measure, stream, branch)`
  (V63). Distinguishes "state was legitimately empty here" from "not cubed yet";
  without it a materialized read reports 0 instead of admitting it doesn't know.
  The stream's watermark is the MAX across its branch rows, and a row's
  existence doubles as the branch-seeded marker.

**Rules before touching these:** never let a read take *data* from the cube that
the facts don't have; never aggregate coarser than a capture (it's the floor that
keeps the tie-back to snapshots and turns working); `dimension.promoted` defines the grain, so
promoting a dim is a **rebuild**, not a schema change. **Anything that deletes
captures or facts must invalidate the affected stream's cube in the same
transaction** — cube rows are frozen at build time and don't cascade (
`prune_dominated_tree_captures` is the one such caller today, and it invalidates
only when it actually dropped something, since it runs on every boot). Written outside the
fact-insert transaction, which is safe only because whole-capture replay is
idempotent and the watermark advances atomically with the rows. Full rationale +
the read's eligibility rules: **`.context/metrics.md`**.


### `test_case_stat` — each test's summary (`V139__test_case_stat.sql`)

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
`on_shell_run`) — the report-less-test-run nudge and post-tool-use
advisories.
Previously fully ephemeral (returned as `additionalContext`, then
lost); persisting gives a reviewer/human-facing answer to "what did oxplow
tell the agent in this request." See `.context/agent-model.md` (Nudge
persistence).

Columns: `id, thread_id (NOT NULL, FK threads ON DELETE CASCADE), kind,
message, trigger (nullable), created_at`, plus `turn_id`, `request_id`
(V24), `cause`, `delivered_at` and `audience` below. Index on
`(thread_id, created_at DESC)`.

- **`thread_id` NOT NULL, `request_id` nullable**: every nudge fires within a
  thread; the request is the one of the turn it belongs to (V24 filled it
  from `turn_id`). The FK is `ON DELETE SET NULL`.
- **`kind`** is open-ended (`report-less-run` | `<namespace>/<advisory>` |
  `configure`, …) — adding a kind needs no migration. Retired kinds keep
  their rows: `commit-hygiene` and `oxplow/coverage-target` no
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
  coalesce(request_id, 0)) WHERE cause IS NOT NULL` (V24) makes a
  redelivered event unable to fire the same nudge twice for one request
  (`record_tx` → `None`). **Delivery is by thread, not by
  cause:** `take_for_agent(thread, budget)` returns the thread's agent
  nudges with no `delivered_at` (oxplow's own first, oldest first, up to
  a character budget), and stamps them in the same transaction —
  so a nudge whose reactor finishes after its hook's response went out
  reaches the agent on the thread's next hook instead of being lost. Rows
  from before V102 were stamped delivered.
- **One-shot marks are durable** (V102; per thread since V9):
  `once_mark(thread_id FK threads CASCADE, request_id FK request CASCADE
  NULL, mark, fired_at)`, unique on `(thread_id, coalesce(request_id, 0),
  mark)` — a mark is kept in a request, or in the thread when
  `request_id` is NULL (`OnceScope`). It holds
  `report-less-run`, `<namespace>/<advisory>` and
  `<namespace>/<advisory>#<row key>`. `claim_once_tx` is the insert (`true`
  the first time); `has_fired` reads it. It replaces the in-memory sets
  (`AdvisoryRunner.fired` and the like), so a
  restart no longer repeats guidance. A mark is separate from the nudge row
  because prompt advisories fire once-per without writing a nudge. No
  store-side retention prune (nudge volume per request is tiny).

### `agent_token_usage` + `agent_token_cursor` — `SqliteTokenUsageStore` (`crates/oxplow-db/src/token_usage_store.rs`, migrations `V35__agent_token_usage.sql`, `V36__agent_token_usage_prompt.sql`)

Per-turn agent token accounting parsed from the session transcript.
On Stop the runtime reads `transcript_path` from the hook
payload, splits the NEW tail into one row per agent turn, and writes them
here. See `.context/agent-model.md` (Token usage capture) for the capture
flow.

`agent_token_usage` columns: `id, stream_id (NOT NULL, FK streams ON
DELETE CASCADE), thread_id (NOT NULL, FK threads ON DELETE CASCADE),
session_id,
agent_kind, model (nullable), prompt (nullable), input_tokens,
output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
message_count, provenance (CHECK `observed`), recorded_at, turn_id, cause`.
Index on `(thread_id, recorded_at DESC)`; a request's totals read the
rows through `turn_id` (index `idx_agent_token_usage_turn`). V102 (P3.2) added `turn_id` (the turn the tokens were spent in) and
`cause` (`UNIQUE WHERE NOT NULL`: the `agent.turn.ended` event whose own
report carried the counts, so a redelivered event counts once).
Transcript rows leave `cause` NULL: one transcript chunk can hold several
turns, and the cursor committing with the rows is what makes their
redelivery a no-op.
`record_batch(rows, cursor)` writes the rows and advances the session's
cursor in **one transaction**, so the cursor never passes bytes whose rows
are missing (or vice versa); `insert_tx` / `set_cursor_tx` are the cores.

- **Per-request totals** read through `turn_id` (`v_request` sums them);
  per-thread totals read `thread_id`, so a row with no turn still counts
  for its thread.
- **`provenance` is always `observed`** — oxplow read the transcript
  directly (no agent-asserted path). **`model`** is the actual per-turn
  model (e.g. `claude-opus-4-8`) so $ cost can be layered on later;
  display is tokens-only today.
- **`prompt` (nullable)** is the human-authored user prompt that
  OPENED the turn — pure observation read from the transcript, never
  generated or sent (no new input surface). Nullable: a turn can be an
  assistant continuation with no fresh prompt at the head of the chunk.
  Stored locally like every other turn artifact (same privacy posture
  as the token counts).
- **`agent_kind`** records who ran the turn. Only Claude is parsed today;
  Codex/Opencode rows aren't written yet (their transcript formats
  differ) — the parser is pluggable per kind.

`agent_token_cursor` columns: `session_id (PK), byte_offset, updated_at`.
The persisted read offset into each session's transcript so successive
Stops only sum the new tail — and a daemon restart never re-sums
already-recorded usage. No FK (the transcript outlives any single row).

Store methods: `record`, `totals_for_thread`, `cursor`/`set_cursor`. There are no IPC reads: the UI
reads `v_token_usage` through pages (bundled's `task-tokens`,
foundation's `thread-activity`, the holder's `usage`), which re-run on `DatasetsChanged`.

### `panel_layout` — `SqlitePanelLayoutStore` (`crates/oxplow-db/src/panel_layout_store.rs`)

V126 (P6.G1). The person's left-nav layout in this project: one row per
panel they've placed (`panel` — a core id like `core:bookmarks` or
`ext:<namespace>/<id>` — `position`, `hidden`, `collapsed`). Local state,
never the repo. `get` returns the placements in order; `set` replaces
them all in one transaction. A panel the table doesn't name shows
expanded after the ones it does, and a named panel that no longer exists
is dropped (`components/Panels/panelLayout.ts` `resolveLayout`). UI-only
RPCs `get_panel_layout` / `set_panel_layout`.

### `ui_preference` — `SqliteUiPreferenceStore` (`crates/oxplow-db/src/ui_preference_store.rs`)

V96. What the person's oxplow remembers for them outside any one tab
(`.context/pages-and-tabs.md` "Page state"): `(scope, key)` → `value_json`
(JSON text), `updated_at`. Scopes: `app` (app preferences); a page's URI
without its params (`oxplow:oxplow/page?page=work`, `…?page=acme/pulls`) or
`resource:<namespace>/<name>` (page memory, shared by every tab of the page); a panel's
URI; `component:<namespace>/<id>` (a custom component's own memory). Anything
else is refused, as is text that isn't JSON or a value over 64 KB, and a
component's keys together stay under 1 MB. Local state, never the repo,
like `panel_layout`. UI-only RPCs `get_ui_preferences` (all, read once at
start by `prefs/remembered.ts`) / `set_ui_preference { scope, key,
value_json | null }`; agents read `v_ui_preference`.

### `bookmark` — `bookmark_store` (`crates/oxplow-db/src/bookmark_store.rs`)

V4. The pages the person starred, each at one scope: `thread`
(`thread_id`), `stream` (`stream_id`) or `project` (neither — the old
localStorage "global" scope; every ref is this project's anyway). A
viewer — a thread and its stream — sees its thread's, its stream's and
the project's, and a ref is bookmarked at most once across those:
`set_tx` takes it out of every scope the viewer sees before inserting, so
bookmarking at another scope moves it. Read through `v_bookmark` (the UI's
`tabs/bookmarks.ts`, a page, the agent); every write is `oxplow.bookmark.set` /
`oxplow.bookmark.remove` (commands.md). They used to live in webview localStorage, where neither pages nor the
agent could see them; those weren't carried over.

### `context_item`, `context_basket` — `context_store` (`crates/oxplow-db/src/context_store.rs`)

V47 (rebuilt from V107's `thread_context_item`, keeping what still
waited). What the person collected for agents while exploring, each item
in one basket: a thread's own (`thread_id`, shown as "This Thread") or a
named basket (`basket_id`; a CHECK keeps exactly one). `kind` is `page`
(`source_uri` the page's canonical URI) or `text` (`text`, from
`source_uri` when known). `note` (V44) is the person's own words on it,
what they want done about it. `around` (V46) is the lines around a `text`
whose source has no lines to read back (a terminal's scrollback).
`stream_id` is the stream it was collected in, so a file it names is read
in that worktree when it is delivered in another stream.

`held_at` (V74) marks an item Put Back returned to the thread's own
basket: the next prompt skips it until Send Next on This Thread clears it
(`release_held_tx`; `hold_tx` sets it again for undo; a move drops it).

`context_basket` holds the named baskets: `name` (unique, NOCASE; never
"This Thread", never starting with `@`, the app's marks for This Thread and
a new basket), and `next_thread_id` (V48), the thread whose next prompt
carries it instead of the thread's own (Send Next; a unique partial index
keeps one per thread, a basket is next for one thread at a time; SET NULL
when the thread goes). They belong to the project, across every stream.
The next prompt to any agent in a thread carries **one** basket — the one
set next while it holds something, else the thread's own
(`waiting_for_thread_tx`) — then `take_delivered_tx` links the turn to the
items (`prompt_mention`), **deletes** them and clears `next_thread_id`, so
the thread sends its own again
(`agent-model.md` "The context basket"): there is no delivered history.
`context_sent_item` (V49) is each thread's **last send**: a copy of what it
carried (`basket_name`, NULL for the thread's own, and the item's fields),
replaced by the next send, gone once put back (`put_back_tx`, which keeps
each item's first `added_at`; read through
`v_context_sent`, one row per thread). Read through `v_context_item` and
`v_context_basket`; every write is an `oxplow.context.*` command
(commands.md).

### `capability_provider` — `SqliteCapabilityStore` (`crates/oxplow-db/src/capability_store.rs`)

V128 (P6b.C2; V15 adds `title`, `source`, `available`, `chosen_by`; V16
the capability's own `capability_title`, `choosable`, `optional`).
Each capability's implementations and their features (`capability`,
`provider` — the primary key — `extension`, NULL for core's,
`features_json`, `active`). Restated whole (`reset`) by the app's
`CapabilityRegistry` (`capabilities.rs`) at boot, when the extensions
change, as an instance starts or stops, and on each reconcile. An
external provider's `provider` is its **instance
id** (P9.B1: a provider's default instance has the provider's id, a
second instance its own — `issues_acme`), the same id its refs and
`contribution_health.contribution` carry. Published as `v_capability_provider`;
see [providers.md](./providers.md).

### `asset_state` — the asset runner (`crates/oxplow-app/src/assets.rs`)

V130 (P7.B1). Each asset's last recompute: `asset` (the primary key —
`metric_cube`, a materialized model's view), `computed_at`, `events_to`
(the event log's highest seq as the recompute began), `snapshot_id` (when
the asset has one), `elapsed_ms`; V143 (P8.B4) adds `mode` (`full` /
`incremental`), `watermark` and `row_count`; V151 adds `definition` (a
materialized model's SELECT, hashed — a clocked asset recorded for
another definition is due at once). Written by the runner after
each recompute; read as `v_asset`. **`asset_failure`** (V152):
an asset's last recompute failure — `failed_at`, `error` — written when
one fails and deleted in the transaction of the next success; `v_asset`
(v3) joins it, so an asset that never succeeded is listed too. See [semantic-layer.md](./semantic-layer.md)
"Assets".

### `collector_run` and `collector_checkpoint` — `SqliteCollectorStore` (`crates/oxplow-db/src/collector_store.rs`)

V65 moves each collector's saved state into `collector_checkpoint` and
gives `collector_run` one row per run. A detail row has `run_id` (the
primary key and a cascading reference to `contribution_run`), `owner`
(the collector's namespace), `id`, and `row_counts_json` for that run.
The migration leaves older per-run counts NULL where they were never
recorded and creates a shared run for a checkpoint that lacks its last
run.

`collector_checkpoint` is keyed by `(owner, id)`. It stores the opaque
`cursor_json`, the highest processed trigger sequence in `last_event_id`,
the newest finished run in `last_run_id`, and the last successful run's
`row_counts_json`. A failed run advances the last-run pointer and keeps
the last good counts and cursor. `record_run_in` writes the shared run,
detail and checkpoint together. The entity runner includes those writes
in the transaction for the collector's output and `collector.synced@2`.

Read history as `v_collector_run`, checkpoint state as
`v_collector_checkpoint`, and standing as `v_collector`. The history
view marks the run named by the checkpoint with `latest = 1`. See
[semantic-layer.md](./semantic-layer.md) "Collectors".

### `effect_run` — `crates/oxplow-db/src/effect_run_store.rs`

V66 gives each **attempt** at an effect's reaction a shared run.
`effect_run` keeps its integer `id`, `effect` (`<namespace>/<id>`),
`event_id`, `event_seq`, `attempt` (from 1), `origin` (`live`, `retry`,
`backfill` or `auto`), `command_run_id`, `proposal_id`, `retry_at` and
`resend_json`. The unique key `(effect, event_id, attempt)` prevents an
attempt from landing twice. The highest attempt number describes the
reaction's current state (`latest_tx` and `v_effect_run.latest`).
`v_effect_run.retryable` identifies attempts that ended failed, denied,
cancelled, invalid or interrupted and have no proposal.

`effect_run.run_id` uniquely references `contribution_run` with a delete
cascade. A claimed attempt begins the shared row in the claim transaction.
A transaction that finishes the attempt also finishes the shared row.
A live attempt has NULL `outcome`. Its recovery ends it as `interrupted`.
An attempt that asked for confirmation records `skipped` and its proposal
ID. [Effect delivery and retry](./extensions-events.md)

### `contribution_run` — `crates/oxplow-db/src/contribution_run_store.rs`

The shared record for a run of contribution code
(`oxplow_domain::run`, [The run context](./extensions-permissions.md)).
It holds the fields common to every run. The command, collector and
effect detail tables reference it directly. A job references it from each
attempt so retries have their own timing, costs and outcome.

- **Identity and cause.** `run_id` is a UUIDv7. `contribution` is the
  contribution's URI. The row records its `kind`, `extension`, `version`
  and `generation` when pinned. `actor_kind` and `actor_id` identify who
  ran it. `stream_id`, `thread_id`, `turn_id`, `session_id` and
  `snapshot_id` are integer anchors. `cause_event_id` identifies the
  causing event, and `mode` says how the run executes.
- **Parent.** `parent_run_id` is set only if the parent's row exists.
  Deleting the parent clears the link. A run without a contribution
  still writes no shared row.
- **Lifecycle.** Commands and collectors use `record_tx` to begin and end
  in one transaction. A run that spans transactions can use `begin_tx`
  and `finish_tx`. A live row has NULL `finished_at`, `outcome` and cost.
  The store refuses a second finish. Startup recovery ends stray live
  rows as `interrupted` with unknown cost. The effect and job ledgers
  recover their claims through their delivery and lease rules.
- **End and cost.** `started_at` and `finished_at` are RFC 3339.
  `outcome` is `ok`, `skipped`, `failed`, `denied`, `cancelled`,
  `invalid` or `interrupted`. The row records `error`, `wall_ms`,
  summed script `cpu_ms`, maximum script `peak_bytes`, `ai_calls`,
  `scopes_json` and `decisions_json`. Inputs, results, credential values
  and prompts stay outside this table. Program errors redact any
  credentials the run received.
- **Readers.** `v_contribution_run` publishes the shared fields, with
  `scopes` and `decisions` for the two JSON columns, and the run's URI.
  `command_run.run_id`, `collector_run.run_id` and `effect_run.run_id`
  name their shared runs. `job_run_attempt.run_id` names the
  attempt's run. `ai_call.run_id` names the run behind a call when one is
  known. The run's model-call count increments when the provider is invoked.
  A cached result keeps its original call link and adds no new call.
- **Retention.** `event_retention::sweep` applies all run retention rules
  in one sweep, in batches of 5,000. It sweeps finished runs only.
  Deleting a run also deletes its detail row through the foreign key.
  - Refused commands (`invalid` or `denied`) age out after 30 days
    (`REFUSED_COMMAND_RUN_DAYS`). Other runs age out after 90 days, with
    successful commands that recorded input kept for undo.
  - Collector runs age out after 90 days. The run named by
    `collector_checkpoint.last_run_id` stays for as long as the checkpoint
    points to it.
  - Effect attempts and their runs stay to prevent delivery or backfill
    from repeating a reaction.
  - Job attempts and their runs stay while the job exists.
  - Runs without a kind ledger follow the 90-day window
    (`CONTRIBUTION_RUN_DAYS`).

  The indexed batch selection took 0.2 ms on 66,535 copied project runs
  at the current cutoffs and 2.4 ms to select a full batch with cutoffs
  advanced 120 days. These measurements cover selection; the current
  indexes are sufficient for this history size.

### `contribution_health` — `SqliteContributionHealthStore` (`crates/oxplow-db/src/contribution_health_store.rs`)

V135 (P7.C1). Each extension contribution's health on this machine, keyed
`namespace` (V6: the namespace it is named in, so it keeps its health when it
moves to a sibling) + `kind` (`provider` | `collector` | `effect`) +
`contribution` (its provider's, collector's or effect's id) — V140 put `kind` in
the key, since a provider and a collector may share an id:
`state` (`ok` / `failing` /
`disabled`), `reason`, `consecutive_failures`, `last_ok_at`,
`last_error`, `mean_ms` (a moving average), `next_due_at`, `updated_at`.
`next_due_at` is when a scheduled contribution should have run again by
— written each minute by the schedulers (`run_due_collectors` for an
enabled `every:` collector; `ProviderRegistry::sync_due` for a
running instance, its earliest collector): the last run (or now, when it
runs now) plus the interval plus one scheduler tick
(`contribution_health::next_due_ms`); `NULL` for one off any schedule (manual,
`on:`, disabled, `syncMinutes: 0`). A rate-limited instance
keeps its last plan. The policy is `oxplow-app/src/contribution_health.rs`:
three failures in a row disable it, the row and `contribution.disabled@1` in
one transaction; `oxplow.contribution.enable` clears it (`contribution.enabled@1`).
`v_contribution_health` adds `dead_letters` (pending dead letters of its
consumer `extension:<namespace>/<contribution>`, or of events whose subject
is `extension:<extension>`) and `fresh` (0 once `next_due_at` has passed, and 0 for a collector whose
last run failed: its rows are the last good run's, not current).
`v_contribution_health` shows `repair_item`: the open task whose title starts
`Repair <extension> <contribution>:`, which the bundled repair effect files (V136
added `repair_item`/`repair_seq` columns for the old core consumer; V58 drops them).

### `provider_collector_state` — `SqliteProviderCollectorStore` (`crates/oxplow-db/src/provider_collector_store.rs`)

V129 (P7.A3). Where each provider instance's collector left off:
`instance`, `collector` (the primary key), `state_json` (the provider's
own opaque `$/state` checkpoint), `status` (`never` / `reading` / `ok` /
`error`), `error`, `last_read_at`, `records` (what reads have delivered).
`checkpoint_tx` runs in the same transaction as the
record events a checkpoint covers, so a read that fails
midway resumes from the last batch that landed; `finish_tx` records the
outcome. Read by `oxplow.provider.sync` and Settings → Integrations; see
[providers.md](./providers.md) "Reading: collectors and sync".

### `command_proposal` — `SqliteProposalStore` (`crates/oxplow-db/src/proposal_store.rs`)

V127 (P6b.A2). A command an agent ran that needs a person's confirmation,
kept until a person decides: the command and input, who proposed it
(`actor_kind`, `actor_id`, `thread_id`, `stream_id`), the confirmation
`preview_json`, `dry_run_json` (what it would have done when proposed;
NULL when it can't be dry-run), and `decision` — `pending`, `approved`
(with the approving run's `command_run_id`), `declined` or `superseded`.
`proposal_key(command, input)` is `config:<key>` for `oxplow.config.set` /
`oxplow.config.unset`, else the command and its input with keys sorted;
`insert_tx` marks pending rows with the same key `superseded` (with
`superseded_by`) and returns which (`Inserted { id, superseded }`, for
`command.proposed@4.supersedes`). `approve_tx` / `decline_tx` decide a
pending row once — a decided one is `Invalid`, a missing one `NotFound`.
An `External` command's approval is the claim trio instead: `claim_tx`
marks it approved with no `command_run_id` before the run, `finish_claim_tx`
names the command run, and `release_claim_tx` makes it pending again
when the run failed. `dry_run_json` is the `Tx` handler's `result` run
confirmed and rolled back: `oxplow.config.set` / `oxplow.config.unset` give `{ key,
before, after, changed }`, a composite `{ result, children: [{ name,
input, result, inverse? }] }`, any other command its own result. No
expiry. Written
inside the bus's transactions; read as `v_command_proposal` (ref
`proposal:<id>`). Events `command.proposed@4`, `command.approved@3`,
`command.declined@2`.

### `page_visit` — `PageVisitStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Append-only event log of in-app page navigations. One row per visit
recorded by `App.handleOpenPage` (skipping `agent`, `new-stream`,
`new-task`).

Rows carry canonical tab ids ([resources.md](./resources.md)); V93 wiped the
pre-canonical ones, and V102 emptied the table again when a visit became the page's identity (the id without its `#`), so no row holds a view. Columns: `id`, `page_kind`, `page_id`, `visited_at` (ISO),
`duration_ms?`, `thread_id?` (added in v3 — nullable so legacy rows and
boot-screen visits with no selected thread still record). A visit is
recorded open when its page is shown (`record_page_visit`, no length) and
finished by `finish_page_visit { id, duration_ms }` when it ends: another
page shown, the thread changed, or the window's `pagehide`. The length is
time with the window visible (`apps/desktop/src/pageVisitTimer.ts`); a
visit whose end the app missed (it quit) keeps NULL. Indexes on
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

The Work panel's "Finished" group is the bundled extension's
`thread_tasks` model: the done tasks a request on the thread worked on
(`v_oxplow_request_link.thread_id`, `v_oxplow_task.closed_at`, whichever
store holds the task) and the knowledge pages it wrote (`v_oxplow_knowledge_touch`, over bundled's
`knowledge_touch_record` model), newest first. "Clear" runs
`oxplow.tasks.clear_finished`, which logs an event the
`finished_cleared` model reads, so entries closed before it are hidden
until something newer lands.

### Comments — dropped (`V45__drop_comments.sql`)

V45 drops the `comment` and `comment_message` tables and their indexes. The
anchored-comment feature is gone; the person's remark for the agent is the
note on a context-basket item (`context_item.note`, V44).
(A shared model's child comments, such as a task's, are not these tables.)

### `search_entry` + `search_fts` — `SqliteSearchStore` (`crates/oxplow-db/src/search_store.rs`, migration `V25__search_index.sql`)

The unified site-wide search index (FTS5/BM25). `search_fts` is a standalone
FTS5 virtual table (`tokenize = 'porter unicode61'`, `prefix = '2 3'`) storing
`(title, body)` for every searchable entity; `search_entry` maps
`(kind, uri, stream_id)` (resource, URI, stream) → the FTS rowid so a single entity can be updated
or removed in place (a `UNIQUE` index over `(kind, uri, COALESCE(stream_id,''))`
enforces identity, treating global rows' `NULL` stream as `''`).

- `kind` holds a resource (`oxplow/knowledge`) that `v_resource` lists as
  `searchable` (oxplow's `oxplow/task`,
  `oxplow/knowledge`, `oxplow/thread`,
  `oxplow/agent_session`, `oxplow/turn`,
  `oxplow/commit`, `oxplow/page`; an
  extension's, P9.D3), or
  `oxplow/file`; `uri` is the thing's URI, and a hit is that URI.
  Every resource but `oxplow/file` is derived from a model by an asset per kind
  (`kind_search.rs`: `plan_restate_tx` decides from each entry's `version`
  — the feed's own `version` column, else its title-and-body hash; V77
  renamed V164's `content_hash` — and `apply_restate_tx` writes only
  entries added, changed or gone); bundled's read `v_oxplow_search_task`
  (every store's tasks) / `v_oxplow_search_knowledge` and the feeds for the other resources named above
  (`models/search_<kind>.sql`; `.context/extensions-events.md` "Resources" says
  how a resource gets one, and the models' docs say
  what each indexes and whether archived work is included). Files are upserted by the `search.index` pump consumer from
  `snapshot.taken` (`indexer.rs`).
- `stream_id` is `NULL` for project-global rows (knowledge pages, a backlog task) and
  the owning stream otherwise. Search filters `stream_id = ?  OR stream_id
  IS NULL`; BM25 weights title above body (`bm25(search_fts, 5.0, 1.0)`).
- The store is a **derived cache**, never a source of truth: those two
  writers are its only ones. `purge_stream_files` is called when a stream
  is archived.
- `sanitize_query` turns arbitrary user input into a safe MATCH expression
  (each token double-quoted + `*` prefix), so junk input can't throw FTS5
  syntax errors. Exposed as the `search` IPC command + MCP `oxplow_search` tool.

## Assignment queues

Each assignee — a thread, the person, nobody — has one queue: the open
items assigned to it, ascending by the task's `rank` field, then by
`created_at` for an item with no rank. `oxplow.tasks.assign` and
`reorder` name a neighbour in the queue (or none: its end); the
command inserts the item there and renumbers the queue from 0. A task row has no order
of its own.

## Status diagrams (text)

```
task:    backlog ─► ready ─► in_progress ─► done
                             ╰─────────► blocked
                             ╰─────────► canceled
```

### Transitions to `in_progress` (server-side guard)

`taskstore.updateItem` rejects any direct jump into `in_progress`
from `canceled` (an explicit "abandoned" state
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
`DatasetsChanged`); subscribers refetch the affected bucket on
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

- **Directory scope** — the `scope` filter param of the `directory` page
  (and of the launcher's `files` page, which passes it on; `all`, `uncommitted`, the default branch's name for the branch's
  changes, the upstream's name for the unpushed ones), in the tab's
  address; its choices come from the `files-scopes` query, which offers only those
  that exist (no branch scope on the default branch, no unpushed scope
  without an upstream) and only on the worktree: on another filesystem or
  revision it offers `all` alone and the page ignores any other value.

## Related

- [ipc-and-stores.md](./ipc-and-stores.md) — adding new stores and IPC.
- [agent-model.md](./agent-model.md) — how the agent acts on this data.
- [git-integration.md](./git-integration.md) — the commit actions (user-driven).

## Registered configuration overrides

Core and extension configuration use the shared registry and `SettingsStore`.
Ordinary values are flat fully qualified keys in `.oxplow/local.yaml`,
`.oxplow/project.yaml` and `global.yaml`; each registered value resolves local,
project, global, then default. Objects and arrays are atomic. Secret values use
the same levels and APIs, with the host routing storage to the OS keychain.
Registry definitions supply title, description, category, schema, default, UI
metadata, restart requirement and secret status. Public inventory datasets expose
redacted values and provenance rather than a separate configuration model.

V40 advances recorded configuration changes to `config.changed@3` (personal
becomes local) and capability switches to `capability.switched@2` (global
provenance is supported); V42 advances those to `capability.switched@3`
(`chosen_by: personal` becomes `local`). Expired payloads remain expired and
untouched; only their version advances. Core registers the newest shapes, as for other core
event migrations.

The old `extension_config` database schema/store is historical persistence only:
runtime extension reads, writes, publication and schema changes do not consult or
mutate it. There is no automatic migration from old configuration files or rows,
no override schema stamp, and no configuration compare-and-swap. Extension nested
snapshots are projections over registered keys; private workflow state continues
to use its separate CAS store above. See [settings.md](./settings.md) and
[extensions-manifest.md](./extensions-manifest.md) for the runtime contracts.

Metric declaration upserts expose caller-transaction helpers for coherent
extension generation publication. Their async wrappers use the same helpers;
measure capture-scope and dimension promotion changes retain their transactional
cube invalidation/fencing. Metric specs, dimensions, measures and available
catalog metadata can therefore commit with candidate views and vocabulary.
Thin callers cannot invoke these write helpers.
