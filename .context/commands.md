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
| `atomicity` | `Tx` (handler runs inside the bus's transaction) or `External` (see below) |
| `effect` | `Write` (the default), `Read` or `Record`. A read runs without an audit row or `command.executed`, so a polling agent doesn't fill the log, and a thread that may not write can still run it (`config.list_keys`, `config.get`). A `Write` is refused to an agent thread that may not write. A `Record` changes oxplow's own records (`work_item.*`): audited like a write, open to any thread, and its handler refuses only a **claim** — opening an effort — when `TxCtx::may_claim` is false (tsk466) |

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
   review). A `Record` command skips that refusal; the gate's answer
   rides into the handler as `TxCtx::may_claim`, and `TxCtx::claim`
   returns `Denied` (rolled back, audited `denied`, not `error`) when the
   run opened an effort the actor may not claim. Filing and editing
   tasks isn't a claim on the worktree; moving one to `in_progress` is.
4. **Confirmation** — when `confirm` requires it and the call isn't
   `confirmed`, `NeedsConfirmation { preview }` and **nothing is written,
   not even an audit row**. An agent's `confirmed` is ignored: it gets
   the preview and asks the person, who runs the command.
5. **Run and record in one transaction**: the handler, a `command_audit`
   row (`crates/oxplow-db/src/command_audit_store.rs`: actor, input,
   outcome, the handler's `result` — V114, so a run's answer, such as a
   merge's conflicts, stays readable after the fact — and inverse), `command.executed@1` in the event log pointing at
   the audit row, the handler's domain events (with `cause` = the
   executed event, and the actor's thread/stream — `Actor::anchors()` —
   filled into any anchor the handler left empty), and the audit row's
   `event_id`. A handler failure
   rolls all of it back and is audited as `error` in a transaction of its
   own.
6. Post-commit: wake the event pump.

A `Read` command stops after step 4: its handler runs in
`Database::read` — a snapshot that is always rolled back, so a write it
makes never lands (nothing would audit it; tsk513) — and the outcome has
`audit_id: None`, `event_id: None`.

`CommandBus::undo(actor, audit_id, confirmed)` loads the row, refuses a
run that didn't complete, was already undone or has no inverse, runs the
inverse through the same pipeline, and marks the row `undone_by` the new
run. The original row is claimed **with** the inverse run
— marked `undone_by` inside the run's transaction for a `Tx` inverse,
or claimed (`undone_by = 0`, pending) before an `External` inverse and
released if it fails — so two concurrent undos can't both apply it
(tsk437 review).

**Agent rules follow the agent.** An `Actor::Lens { on_behalf_of }`
whose chain ends at an agent (`Actor::is_agent_driven`) gets the agent
policy and can never confirm, exactly like the agent.

**An `External` run whose recording fails** (its effects already
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

## `Tx` vs `External`

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
retries reaches the caller, as `Busy` (RPC `BUSY`). An **`External`**
handler (P5.A1) is an async call against a system the bus doesn't own —
a VCS, a provider process, a gauge script — whose state can't join the
bus's transaction; the bus audits it after it returns (a failure to
record is logged, never reported as the run failing); its audit row
holds the handler's `result` like a `Tx` run's. It is the right
kind for exactly those commands, not a shortcut: `CommandBus::
external_commands()` is pinned by `the_external_commands_are_the_reviewed_ones`,
and adding one means naming its system in the summary. Registering a
handler whose kind disagrees with the spec's `atomicity` is refused, as
is a second command of the same name.

## Commands so far

| Command | Handler | Notes |
|---|---|---|
| `work_item.transition { ref, to }` | `Tx` over `oxplow_db::task_store::set_status_tx` (`commands/work_item.rs`, P2.6.3) | all invokers; undoable (inverse restores the prior status). The row, the effort open/close, `work_item.transitioned` and `effort.*` commit with the audit, all caused by `command.executed`; the effort's snapshot pin is the effort-lifecycle pump consumer's. |
| `work_item.create { title, description?, parent_ref?, status?, priority?, thread? }` | `Tx` over `task_store::insert_logged_tx` (`commands/work_item.rs`, tsk463) | all invokers; not undoable (that would be deleting a task). The row at the end of its list (`next_sort_index_tx`), `work_item.created@1 { work_item, status, effort? }`, and — filed `in_progress` on a thread — the effort, all caused by the run; an agent's task is authored `agent`. Absent `thread` files onto the backlog. The result is the task plus its `ref`. |
| `work_item.update { ref, title?, description?, priority?, parent_ref?, status? }` | `Tx` over `task_store::update_with_status_tx` (`commands/work_item.rs`) | all invokers; undoable (the inverse restores exactly the fields and status given). Fields and status commit together: `work_item.edited@1 { work_item, fields }` for the fields, then the status move with everything `work_item.transition` implies. A refused run writes nothing. |
| `work_item.link { ref, target, link_type, thread? }` / `work_item.comment { ref, body }` | `Tx` over `task_satellite::create_link_tx` / `add_task_note_tx` (`commands/work_item.rs`, P5.C2) | all invokers; not undoable. A typed link (made in `thread`, the caller's by default) or a note on the task, with its `page_ref` edges, logging `work_item.linked@1` / `work_item.commented@1` caused by the run. They replaced MCP `link_tasks`. |
| `effort.open { work_item, thread? }` / `effort.close { effort, summary? }` | `Tx` over `effort_store::start_tx` / `finish_tx` (`commands/effort.rs`, P2.6.4) | all invokers; not undoable. For a work item whose provider doesn't open its own effort (`work_item:linear:ENG-12`): a registered provider declaring `in_progress_opens_effort` — oxplow's tasks, whose effort follows their status — is refused; an unregistered provider's item takes one. `thread` defaults to the caller's and must be its stream's working (active) thread; an agent may name only a thread in its own stream (and close only its stream's efforts). A second open on the same item is refused naming the open effort. Logs `effort.opened` / `effort.closed` caused by the run; the snapshot pin is the effort-lifecycle pump consumer's. |
| `config.list_keys {}` / `config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value, `human_only` |
| `config.set { key, value }` / `config.unset { key }` | `Tx`: validate against the key's schema, take the new document through the loader's own validation (`oxplow_config::keys::with_key`); after commit, write the file and swap the in-memory config | undoable (inverse restores the prior value or unsets); logs `config.changed@1 { key, before, after }`; `after_commit` broadcasts `ConfigChanged`; a **human-only key** (`HUMAN_ONLY_KEYS`: `ai`, `agents`, `agent`, `agentModels`, `acpAgents`, `extensionInstances`, `lsp`, `collection`, `extensions`, `gauges`, `agentPromptAppend` — each runs a program, picks the model, enables code, or steers every agent; a test fails if a key documented as running programs or steering agents isn't listed) needs a person's confirmation per input |
| `metric.enable { keys, enabled }` | `Tx`: turns metrics on or off — computes the new `metrics:` list with `MetricsService::apply_metric_enabled` (a bundled gauge is off until a `use:` names it; a producer/plugin metric is on until an `enabled: false` marker) and hands it to `config.set`'s core; an unknown key (not in `metric_catalog`) is refused | undoable (restores the prior list); logs `config.changed@1 { key: metrics, … }`; the reseed follows `ConfigChanged` |
| `metric.record { key, value, subject?, dims?, stream? }` | `Tx` over `fact_store::record_facts_tx` (`commands/metric.rs`, P4.8) | not undoable. An asserted fact on the metric's source measure, stamped to match its filter, anchored to the stream's latest snapshot; the fact and the audit commit together. A formula or `count` metric, an unknown key, or another stream (for an agent) is refused. After commit: clears the fact memo, emits `MetricSamplesChanged` for the measure |
| `metric.run { key, stream? }` / `metric.rebuild { force }` | `External` over `MetricsService::run_metric_by_key` / `rebuild_baseline` | not undoable. Run one gauge now, or every gauge's whole-tree baseline. They drive snapshot captures and gauge scripts, which own their own transactions |
| `vcs.commit` / `vcs.stage` / `vcs.discard` / `vcs.fetch` / `vcs.pull` / `vcs.push` / `vcs.merge` / `vcs.checkout_branch` / `vcs.rename_branch` / `vcs.delete_branch` / `vcs.resolve_conflict`; `git.rebase` / `git.cherry_pick` / `git.revert` / `git.ignore` | `External` over the `Vcs` trait (`git.*`: the git provider's own ops) (`commands/vcs.rs`, P5.B6) | a person's only (`human`; agents run `git` in their terminal), not undoable. Each takes the `stream` it acts on, resolved strictly (an unknown stream is refused, never the primary's). `vcs.discard`, `vcs.merge`, `vcs.delete_branch`, `git.rebase` and `git.revert` are `Destructive` (confirmed). The result — and the audit row's — is the VCS's `OpOutcome { success, log, conflicts, auto_resolved }` (`vcs.commit`: `{ success, revision }`). After a run the stream's `WorkspaceChanged` (and `VcsRefsChanged`; every stream's after a fetch, push, rename or delete) is announced. See [vcs.md](./vcs.md) |
| `knowledge.write_page { slug, title?, body, verified_refs?, removed_refs? }` / `knowledge.delete_page { slug }` / `knowledge.link { page, target }` / `knowledge.resync { slug }` | `Tx` over `knowledge::write_page_tx` / `delete_page_tx` (`crates/oxplow-app/src/knowledge.rs`, P5.C3) | all invokers, `Record` (a read-only thread captures too); not undoable; `delete_page` is Destructive. Validates the slug and every `[[link]]` (refused, named), restates the `wiki_page` row and its pinned `page_ref` edges, logs `knowledge.page.written@1` / `knowledge.page.deleted@1` with the actor's anchors; after commit writes (or removes) `.oxplow/wiki/<slug>.md` and announces `WikiPagesChanged`. See [knowledge.md](./knowledge.md) |
| `<provider>.<name>` (an enabled external provider's declared commands: `fake.create`, …) | `External` over the provider process's `invoke` (`providers/registry.rs`, P5.D3) | registered while its instance is enabled (and removed when it stops), all invokers, `Experimental`; `confirm`, `effect` and `undoable` as the provider declares (its `inverse` becomes `<provider>.<command>`). The events its `invoke` returns are logged caused by the run — only types it declares, and a `work_item.recorded` only for its own items. See [providers.md](./providers.md) |
| `provider.enable { instance }` | `External` over the provider registry (`providers/registry.rs`, P5.D4) | a person's only, not undoable. Enables an extension provider's instance on this machine again — clearing an automatic disable — and reconciles, so it starts when `extensionInstances` enables it. Logs `provider.enabled@1 { instance }`; the result is the instance's view with its health. See [providers.md](./providers.md) |
| `metric.scaffold { key, title?, language?, glob? }` | `Tx`, `Read` | a starter gauge script and the measure + gauge + metric entries; writes nothing (the agent adds the entries with `config.set`) |

The `work_item.*` commands are the oxplow provider's
([work-items.md](./work-items.md)): each names its task by its canonical
ref (`work_item:oxplow:tsk42`) and refuses another provider's, naming the
registered ones (`no work-items provider \`linear\`; registered:
oxplow`).

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
connection can't file or change a task, and a queued thread can file,
edit and finish tasks but not move one to `in_progress` (tsk466); RPC
`create_task` / `update_task` / `upsert_task` run them as
`Actor::Human`. `TaskService::update`
(no actor) still logs every status change, with source
`system:task_service`, but isn't audited.
**Config is written only by `config.*`** (tsk515). The Settings page's
typed IPC setters (`set_agents`, `set_agent_prompt_append`,
`set_agent_model`, `set_snapshot_retention_days`,
`set_snapshot_max_file_bytes`, `set_generated`, `set_extension_enabled`)
and `enable_metrics` each run `config.set` / `config.unset` as
`Actor::Human`, confirmed — the person's click is the confirmation a
person-only key asks for — through `oxplow_rpc::commands::config::set_key`.
`config_service` only reads. `only_the_config_commands_write_project_yaml`
scans the crates for any other `write_project_config` call. A key whose
new value must reach something running reacts to the `config.changed`
event on the pump, reading the value from the event's `after` (the pump
can see the event before the after-commit swap): `generated` →
`config_reactors::WorkspaceFilterConsumer` updates the snapshot captures'
filter, so an agent's change applies like the person's.
**The person's way onto the bus** (P5.A1): RPC `run_command { name,
input, confirmed }` and `undo_command { audit_id, confirmed }`
(`oxplow_rpc::commands::bus`, `Actor::Human`; desktop `runCommand` /
`undoCommand` in `api.ts`). A call that needs confirmation comes back
`NEEDS_CONFIRMATION` and the UI asks, then calls again with `confirmed`.
A typed IPC setter is a convenience over one command; anything new the
UI writes goes through `run_command`. Parity: `both("run_command")`,
`ui("undo_command")`. The thrown error keeps its IPC code
(`IpcCallError.code`, `needsConfirmation(e)` in `ipc-error.ts`), and
the UI asks with **`CommandConfirm`** (`components/CommandConfirm.tsx`:
the command's summary from RPC **`get_command { name }`**, destructive
ones marked; Run focused, Escape cancels). A command's `input_schema`
renders as a form with **`SchemaForm`** (`components/SchemaForm/`:
schemars' shapes — `$defs`/`$ref`, `Option<T>`, enums, nested objects,
string lists — else a JSON field; Enter submits, Escape resets, submit
disabled while a field has a problem; model in `schemaFormModel.ts`).
P6.B2.

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
