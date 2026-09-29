# Commands: the one write path

What this doc covers: the command bus — what a command declares, who
may run it, the pipeline every run goes through, the audit and undo,
and how a new command is added. Target design:
[target-architecture.md](./target-architecture.md) §7. Built in
P1.8–P1.10 (tsk410–412): the bus, `config.*`, and the generic MCP
surface (`list_commands` / `run_command`) with caller identity.

## What a command is

A command is a typed operation named `<capability|plugin>.<verb>`
(`work_item.transition`, `config.set`) that declares
(`oxplow_domain::commands::CommandSpec`):

| Field | Meaning |
|---|---|
| `input_schema` | JSON Schema; the bus validates the input first and names the failing field |
| `invokers` | which surfaces may run it: `human`, `agent`, `lens` |
| `confirm` | `Never`, `Always`, or `Destructive` — a person confirms; an agent never can |
| `undoable` | the handler returns an inverse call that `undo` applies |
| `lifecycle` | `Stable` / `Experimental` |
| `atomicity` | `Tx` (handler runs inside the bus's transaction) or `BestEffort` (see below) |
| `effect` | `Write` (the default) or `Read`: a read runs without an audit row or `command.executed`, so a polling agent doesn't fill the log, and a thread that may not write can still run it (`config.list_keys`, `config.get`) |

`Actor` is who runs it: `Human`, `Agent { thread_id, stream_id }`,
`Lens { lens_id, on_behalf_of }`, `System`. Its `source()` (`human`,
`agent:thr3`, `lens:acme/x`) is what the event and audit record.

## The pipeline (`crates/oxplow-app/src/commands/mod.rs`)

`CommandBus::run(actor, name, input, confirmed)`:

1. **Validate** the input against the schema → `Invalid { field, message }`
   (audited as `invalid`).
2. **Invoker check** — the spec's `invokers` must admit the actor's surface
   → `Denied` (audited).
3. **Agent policy** — for an agent, `AgentPolicy::check_command(thread,
   spec, may_write)` (`agent_policy.rs`) → `Denied` (audited). A `Write`
   command needs a thread that may write: the bus asks its `WriteGate`
   (`Services` wires "the thread exists and is its stream's writer"), so
   a queued or closed thread can read but not change state (tsk437
   review).
4. **Confirmation** — when `confirm` requires it and the call isn't
   `confirmed`, `NeedsConfirmation { preview }` and **nothing is written,
   not even an audit row**. An agent's `confirmed` is ignored: it gets
   the preview and asks the person, who runs the command.
5. **Run and record in one transaction**: the handler, a `command_audit`
   row (`crates/oxplow-db/src/command_audit_store.rs`: actor, input,
   outcome, inverse), `command.executed@1` in the event log pointing at
   the audit row, the handler's domain events (with `cause` = the
   executed event, and the actor's thread/stream — `Actor::anchors()` —
   filled into any anchor the handler left empty), and the audit row's
   `event_id`. A handler failure
   rolls all of it back and is audited as `error` in a transaction of its
   own.
6. Post-commit: wake the event pump.

A `Read` command stops after step 4: its handler runs on a connection
and the outcome has `audit_id: None`, `event_id: None`. A `Read` must
not write — nothing records it.

`CommandBus::undo(actor, audit_id, confirmed)` loads the row, refuses a
run that didn't complete, was already undone or has no inverse, runs the
inverse through the same pipeline, and marks the row `undone_by` the new
run. The original row is claimed **with** the inverse run
— marked `undone_by` inside the run's transaction for a `Tx` inverse,
or claimed (`undone_by = 0`, pending) before a `BestEffort` inverse and
released if it fails — so two concurrent undos can't both apply it
(tsk437 review).

**Agent rules follow the agent.** An `Actor::Lens { on_behalf_of }`
whose chain ends at an agent (`Actor::is_agent_driven`) gets the agent
policy and can never confirm, exactly like the agent.

**A `BestEffort` run whose recording fails** (its writes already
committed in the service's own transaction) is reported as done and
unrecorded — `audit_id: None`, logged at error level — never as an
error, which would claim the change didn't happen.

A command whose confirmation depends on the input sets
`Command::with_confirm_for(fn(&input) -> Confirm)`; the bus consults it
in step 4. A handler that needs the UI to refetch returns
`HandlerOutput::after_commit`, which the bus runs once the transaction
has committed — the in-memory `OxplowEvent` broadcast belongs there,
never inside the handler.

**A `Tx` handler must be pure** (tsk442): it can run more than once,
because `Database::transaction` retries its closure on SQLITE_BUSY. Only
database writes on the given connection belong inside; anything outside
the database (a file, in-memory state) goes in `after_commit`, which must
tolerate failure since the run is already recorded. `config.set` is the
model: the handler validates and computes before/after; `after_commit`
re-applies the key to the config as it is then, writes project.yaml and
swaps memory under one lock, so concurrent sets of different keys both
survive.

## `config.*` and the key registry

`.oxplow/project.yaml`'s vocabulary is one registry,
`oxplow_config::keys` (`config_keys()`): the JSON Schema generated from
the file's own shape (`RawConfig`, `deny_unknown_fields`), whose
top-level properties are the keys, each with its field doc and value
schema. `write_project_config` renders only those keys and copies any
other top-level key through — deriving the managed set from the schema
is what fixed `metricRetentionDays`, `metricDetailMaxPerProducer`,
`metricDetailRetentionDays` and `iconTint` silently reverting (they were
missing from the old hand-kept `MANAGED_KEYS` list, so their on-disk
value came back as an "extra" over the one just written). Adding a field
to `RawConfig` makes it managed, documented and settable at once.
`set_zones` is gone (tsk392): `zones` is just a key.

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent, the launcher for the human.

## `Tx` vs `BestEffort`

A `Tx` handler is `Fn(&TxCtx, Value) -> HandlerOutput` and composes
into the bus's transaction, so its writes, the audit and the events
commit or roll back together — the target shape. `TxCtx { conn, actor,
events }` carries an `EventCtx` whose `source` is the actor's and whose
`cause` is the run's `command.executed` id — fixed before the handler
runs — so store cores the handler calls (`set_status_tx`,
`effort_store::start_tx`) log their own events as this run's. Those
events precede `command.executed` in `seq` (it's appended after the
handler, with the audit); `HandlerOutput.events` follow it. A handler
must stay pure: `Database::transaction` retries it on SQLITE_BUSY — and
a handler that hits one itself returns `CommandError::Busy` (the
`From<DomainError::Busy>`; map SQLite errors with `oxplow_db::map_sql_err`),
which the bus turns back into a retry; only a busy that outlasts the
retries reaches the caller, as `Busy` (RPC `BUSY`). A
`BestEffort` handler is an async call into a pre-existing service that
owns its own transactions, audited after it returns; it
exists only for handlers that predate the bus, and
`CommandBus::best_effort_count()` is asserted by a test so the number
trends to zero. Registering a handler whose kind disagrees with the
spec's `atomicity` is refused, as is a second command of the same name.

## Commands so far

| Command | Handler | Notes |
|---|---|---|
| `work_item.transition { id, to }` | `Tx` over `oxplow_db::task_store::set_status_tx` (`commands/work_item.rs`, P2.6.3) | all invokers; undoable (inverse restores the prior status). The row, the effort open/close, `work_item.transitioned` and `effort.*` commit with the audit, all caused by `command.executed`; the effort's snapshot pin is the effort-lifecycle pump consumer's. |
| `work_item.create { title, description?, parent_id?, status?, priority?, thread? }` | `Tx` over `task_store::insert_logged_tx` (`commands/work_item.rs`, tsk463) | all invokers; not undoable (that would be deleting a task). The row at the end of its list (`next_sort_index_tx`), `work_item.created@1 { work_item, status, effort? }`, and — filed `in_progress` on a thread — the effort, all caused by the run; an agent's task is authored `agent`. Absent `thread` files onto the backlog. |
| `work_item.update { id, title?, description?, priority?, parent_id?, status? }` | `Tx` over `task_store::update_with_status_tx` (`commands/work_item.rs`) | all invokers; undoable (the inverse restores exactly the fields and status given). Fields and status commit together: `work_item.edited@1 { work_item, fields }` for the fields, then the status move with everything `work_item.transition` implies. A refused run writes nothing. |
| `effort.open { work_item, thread? }` / `effort.close { effort, summary? }` | `Tx` over `effort_store::start_tx` / `finish_tx` (`commands/effort.rs`, P2.6.4) | all invokers; not undoable. For a work item that isn't an oxplow task (`work_item:linear:ENG-12`) — an oxplow task's effort follows its status, so its refs are refused. `thread` defaults to the caller's and must be its stream's working (active) thread; an agent may name only a thread in its own stream (and close only its stream's efforts). A second open on the same item is refused naming the open effort. Logs `effort.opened` / `effort.closed` caused by the run; the snapshot pin is the effort-lifecycle pump consumer's. |
| `config.list_keys {}` / `config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value, `human_only` |
| `config.set { key, value }` / `config.unset { key }` | `Tx`: validate against the key's schema, take the new document through the loader's own validation (`oxplow_config::keys::with_key`); after commit, write the file and swap the in-memory config | undoable (inverse restores the prior value or unsets); logs `config.changed@1 { key, before, after }`; `after_commit` broadcasts `ConfigChanged`; a **human-only key** (`HUMAN_ONLY_KEYS`: `ai`, `agents`, `agent`, `agentModels`, `acpAgents`, `lsp`, `collection`, `extensions`, `gauges`, `agentPromptAppend` — each runs a program, picks the model, enables code, or steers every agent; a test fails if a key documented as running programs or steering agents isn't listed) needs a person's confirmation per input |

**Callers.** Every task edit or status change made for someone is a
command, through `oxplow_app::task_writes`: `create` runs
`work_item.create`, `update` runs `work_item.update` (fields + status,
atomic), `set_status` runs `work_item.transition`, each settling the
effort-lifecycle consumer after a status move; `upsert` inserts a new row through the create path
(`insert_logged`) and edits an existing one with `update` (title,
description, priority, parent, status — not its thread or position). MCP
`create_task`, `file_epic_with_children`, `update_task`, `complete_task`,
`dispatch_task`, `upsert_task` and `transition_tasks` run them as `Actor::Agent` with the caller's verified
thread and stream (`McpCaller` — see "MCP identity"), so an anonymous
connection (or a queued thread) can't file or change a task; RPC
`create_task` / `update_task` / `upsert_task` run them as
`Actor::Human`. `TaskService::update`
(no actor) still logs every status change, with source
`system:task_service`, but isn't audited.
`CommandError` → `McpError` mapping lives in `command_error` (oxplow-mcp):
invalid/denied/unknown are the caller's to fix, `NeedsConfirmation` tells
the agent to ask the person, `Failed` is internal.

## Exposure to agents (MCP)

Agents reach every command through two generic tools — `list_commands`
(the specs the calling agent may run, with `input_schema`, `summary`,
`confirm`, `undoable`) and `run_command { name, input }` (the outcome:
`result`, `audit_id`, `event_id`, `inverse?`). Extensions never add MCP
tools. `transition_tasks` is `run_command("work_item.transition")` per
id.

**Caller identity.** Every harness carries the acting thread on the
HTTP request, and `oxplow_mcp::McpCaller::from_parts` reads it from the
`http::request::Parts` rmcp attaches to each tool call — the
`X-Oxplow-Thread` / `X-Oxplow-Stream` headers (ACP: `McpHttp.headers`;
opencode: `{env:OXPLOW_THREAD_ID}` in its config headers; Claude: a
per-thread `mcp-config.<thread>.json` with the literal headers, since
Claude's MCP config reads no env vars), or `?thread=…&stream=…` on the
endpoint URL (Codex, whose config has no per-session headers). A
connection with neither is an anonymous agent: it may list, and
`run_command` / `transition_tasks` refuse it — no run without an actor
to audit it to. **The header is a claim, not a proof**:
`OxplowMcp::verified_actor` resolves the thread and refuses an unknown
thread or a stream header that isn't the thread's stream, and the actor
carries the thread's real stream.
The wire test `crates/oxplow-control-plane/tests/mcp_wire.rs` proves the
headers reach `command.executed`'s `source = agent:thr…`.

## Adding a command

1. Define the input as a Rust struct with `JsonSchema`
   (`deny_unknown_fields`) and derive the spec's `input_schema` from it.
2. Write the handler — `Tx` unless it must call a pre-existing service.
   Return the inverse when the spec says `undoable`, and any domain
   events as typed envelopes (`Envelope::typed::<T>`).
3. Register it in `Services::new` (`commands.register(...)`), which
   refuses name collisions.
4. Route the existing RPC/MCP entry points through `commands.run(...)`
   rather than calling the service directly, so the human's and the
   agent's runs are audited alike.
5. Tests: the input schema rejection names the field; the invoker and
   confirmation rules hold; the audit row and `command.executed` share
   the transaction; undo applies the inverse.
