# Commands: the one write path

What this doc covers: the command bus — what a command declares, who
may run it, the pipeline every run goes through, the audit and undo,
and how a new command is added. Target design:
[target-architecture.md](./target-architecture.md) §7. Built in P1.8
(tsk410); the generic MCP surface (`list_commands` / `run_command`)
and `config.*` follow in P1.9–P1.10.

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
   spec)` (`agent_policy.rs`) → `Denied` (audited). Today it enforces the
   agent gate; the write guard and filing apply to worktree edits, which
   no command performs yet.
4. **Confirmation** — when `confirm` requires it and the call isn't
   `confirmed`, `NeedsConfirmation { preview }` and **nothing is written,
   not even an audit row**. An agent's `confirmed` is ignored: it gets
   the preview and asks the person, who runs the command.
5. **Run and record in one transaction**: the handler, a `command_audit`
   row (`crates/oxplow-db/src/command_audit_store.rs`: actor, input,
   outcome, inverse), `command.executed@1` in the event log pointing at
   the audit row, the handler's domain events (with `cause` = the
   executed event), and the audit row's `event_id`. A handler failure
   rolls all of it back and is audited as `error` in a transaction of its
   own.
6. Post-commit: wake the event pump.

`CommandBus::undo(actor, audit_id, confirmed)` loads the row, refuses a
run that didn't complete, was already undone or has no inverse, runs the
inverse through the same pipeline, and marks the row `undone_by` the new
run.

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent (P1.10), the launcher for the human.

## `Tx` vs `BestEffort`

A `Tx` handler is `Fn(&Connection, &Actor, Value) -> HandlerOutput` and
composes into the bus's transaction, so its writes, the audit and the
events commit or roll back together — the target shape. A `BestEffort`
handler is an async call into a pre-existing service that owns its own
transactions (`TaskService::update`), audited after it returns; it
exists only for handlers that predate the bus, and
`CommandBus::best_effort_count()` is asserted by a test so the number
trends to zero. Registering a handler whose kind disagrees with the
spec's `atomicity` is refused, as is a second command of the same name.

## Commands so far

| Command | Handler | Notes |
|---|---|---|
| `work_item.transition { id, to }` | `BestEffort` over `TaskService::update` (`commands/work_item.rs`) | all invokers; undoable (inverse restores the prior status). The service logs `work_item.transitioned` itself. |

**Callers.** MCP `transition_tasks` runs one `work_item.transition` per
id as `Actor::Agent` (thread identity arrives in P1.10); RPC
`update_task` routes a status-only change through the bus as
`Actor::Human` and applies any other field change directly.
`CommandError` → `McpError` mapping lives in `command_error` (oxplow-mcp):
invalid/denied/unknown are the caller's to fix, `NeedsConfirmation` tells
the agent to ask the person, `Failed` is internal.

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
