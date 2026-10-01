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
| `confirm` | `Never`, `Always`, or `Destructive` — a person confirms; an agent never can. A `Read` command may not ask (`Command::new` / `with_confirm_for` refuse it): it runs unrecorded, so nothing would resolve its proposal |
| `undoable` | the handler returns an inverse call that `undo` applies |
| `lifecycle` | `Stable` / `Experimental` |
| `atomicity` | `Tx` (handler runs inside the bus's transaction), `External`, or `Dispatch` — one or the other, decided per input (see below) |
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
   `confirmed`, **nothing runs and no audit row is written**: a person
   (or the system) gets `NeedsConfirmation { preview }`; an
   agent-driven run (an agent, or a lens acting for one — whose
   `confirmed` is ignored) is kept as a **proposal** and gets `Proposed
   { proposal, preview }` (see "Proposals", P6b.A3). The answer
   rides into the handler as `TxCtx::confirmed` (and step 3's gate as
   `TxCtx::may_write`): a handler that learns only while running that a
   confirmation is needed — a composite whose child asks — raises
   `NeedsConfirmation` itself, and the bus treats it as this step would
   (rolled back, nothing audited; a person asked, an agent's run
   proposed) (P6b.A1). Only a plain call is proposed: an agent's
   **undo** that needs a person is `Denied` (a proposal is a plain call
   and would lose the row it undoes).
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
(tsk437 review). The one that loses answers `Invalid` ("audit row N was
already undone", `lost_race`) and leaves **no audit row**: it didn't
fail, it was beaten to it. An approval's lost race is the same.

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

## Composition: `command.sequence` and `run_nested` (P6b.A1)

A composite runs several `Tx` commands as **one run**:
`CommandBus::run_nested(ctx, parent_spec, calls)` (`commands/mod.rs`) is
the one mechanism, and `command.sequence { calls: [{ name, input }] }`
(`commands/compose.rs`) is the core command whose handler is exactly
that; an extension's own command (P6b.B2) is "run the script to get
`calls`, then `run_nested`". It first makes a pass that writes nothing:
every call must name a `Tx` command (an `External` one can't join the
transaction — "composes Tx commands only" — nor a `Dispatch` one whose
route for that input is external, the refusal naming the system
(`provider \`linear\``); nor a `Read` one — a
sequence composes commands that write), its input must fit (a problem
is reported at `/calls/<i>/input/…`), and its **own** `invokers`, the
agent policy (with the parent's `may_write`) and its `confirm` apply, so
a composite never widens what its children allow; a child that asks
makes the parent ask (`Preview { command: <parent>, destructive }`)
unless the run was confirmed. Then every handler runs on the parent's
`TxCtx` one level deeper (`TxCtx.depth`); a composite more than
`MAX_NESTING` (8) deep is `Invalid` ("does a command compose itself?")
instead of recursing until the stack overflows, and the whole run rolls
back. The parent has the **one audit row** and `command.executed`
(children are not audited separately — a child row would be an
independently undoable unit fighting the parent's inverse); its
`result` is `{ result, children: [{ name, input, result, inverse? }] }`;
the children's events ride out on the parent's `HandlerOutput.events`,
so they are caused by the parent's `command.executed`; their
`after_commit`s chain in order. The inverse is the children's inverses,
**reversed**, as a `command.sequence` — or none when a child has none —
so `undo` needs nothing new, and a child whose inverse asks makes the
undo ask. A child's `Busy` propagates and the bus retries the whole
parent (handlers are pure).

## Proposals: an agent's run that needs a person (P6b.A3)

A run an agent can't confirm is kept for a person instead of being
refused (`CommandBus::unconfirmed`):

1. **Dry run** — a `Tx` handler runs with `confirmed: true` in a write
   transaction that is always rolled back (`Database::rehearse`) — for a
   `Dispatch` command, only when its route for the input is `Tx`; its
   `result` is what it would have done (`config.set`: `{ key, before,
   after }`; a composite: its `children`). Its events and `after_commit`
   are dropped, so nothing reaches a file. An `External` handler never
   runs: its proposal has no dry run. A dry run that fails is the run's
   failure (audited like one), not a proposal.
2. **Kept** — one transaction inserts the `command_proposal` row
   ([data-model.md](./data-model.md); `proposal_key` supersedes a pending
   proposal for the same config key or the same command and input —
   whichever thread proposed it — and `insert_tx` returns which) and
   logs `command.proposed@1 { proposal, command, actor_kind, actor_id?,
   destructive, supersedes? }` with the actor's anchors, `supersedes`
   naming the replaced proposals so the replacement is never silent.
3. **`Proposed { proposal: "proposal:N", preview, supersedes }`** — IPC
   code `PROPOSED`; MCP `run_command` turns it into a *successful*
   result `{ kind: "proposed", proposal, message }` (`proposed_message`: it waits in
   Approvals and on the setting's row; "It replaces proposal:M" when it
   did; tell the person; don't run it again). Other MCP
   tools that run a command report the same message as an error.

**Deciding** is a person's only (`Actor::Human`; an agent, a lens or the
system is `Denied`), and a proposal is decided once (`Invalid` after):

- **`CommandBus::approve(actor, id)`** runs the proposal's command as the
  person, confirmed, through the whole pipeline (`RunOrigin::Approval`).
  A `Tx` run marks the row approved with its `audit_id` and logs
  `command.approved@1 { proposal, command, audit_id }` (caused by its
  `command.executed`) in the run's own transaction; an `External` run
  claims the row first (approved, no audit row: `claim_tx`), names its
  audit row when recorded (`finish_claim_tx`) and releases the claim if
  it fails (`release_claim_tx`) — the same pattern as an `External`
  undo's claim. A run that fails leaves the proposal pending; a
  concurrent approval that loses answers `Invalid` ("proposal:N was
  already decided") with no audit row.
- **`CommandBus::decline(actor, id)`** marks it declined and logs
  `command.declined@1`; nothing runs, no audit row.

A custom component's frame invokes a command through UI RPC
`invoke_component_command` — as the lens acting for the person, only a
command its component declares ([extensions.md](./extensions.md),
"Custom components").

UI RPC **`decide_proposal { proposal, approve }`** (`ui` in surface
parity; desktop `decideProposal`) returns the approving run's outcome, or
`null` for a decline. A proposal's dry run is a snapshot at proposal
time; the approval re-runs for real and the audit row records what
happened.

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

**Every key has an entry** (P6.H1): `oxplow_config::config_entries`
gives each key its value — the file's, or the default's — and whether the
file sets it; `render_project_config` is the set ones, and a test holds
the entries to exactly the schema's keys. The single-agent `agent` key is
gone: `agents` is the one key (an old file's `agent:` is refused as an
unknown field). Every entry's YAML comes from `oxplow_config::to_yaml`,
the one bridge from a config type to YAML, by way of JSON text: serde_json's
`arbitrary_precision` makes a number inside a `serde_json::Value` (an
extension instance's `config`) a private struct under `serde_yaml::to_value`,
which once wrote `{$serde_json::private::Number: '5'}` into the committed
file (P6 review, tsk599); `keys.rs`'s `yaml_to_json` is the same bridge
the other way.

**Settings is a view** (`crates/oxplow-app/src/effective_config.rs`, UI
RPC `effective_config`): one `EffectiveSetting { key, doc, value, origin,
extension?, humanOnly, schema }` per project key (`project` when the file
sets it, else `default` with the default's value), per AI role
(`ai.roles.<role>`: `project` when the project overrides it, `global`
from `ai.yaml`, `default` when unbound; person-only), and per metric and
dimension from the global manifests (`global`) and enabled extensions
(`extension`). `SettingsPage` lists them grouped and searchable
(`pages/settingsModel.ts`), each with **Ask the Agent to Change This**
(a prompt naming the key, its doc and its value, inserted, never sent) —
a person-only key's `config.set` by the agent becomes a proposal the
person approves. Direct
controls remain only for person-only settings (agents, AI, language
servers, extensions, integrations, programs); `set_snapshot_retention_days`
and `set_snapshot_max_file_bytes` went with their editors. The view
re-reads on `ConfigChanged`, the one signal for every config write: a
`config.set`, and `set_ai_role` (a role's binding is an `ai.roles.<role>`
row), which emits it too (P6 review, tsk607).

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent, the launcher for the human.

## `Tx`, `External` and `Dispatch`

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
a VCS, a provider process, a gauge script, the lens files on disk —
whose state can't join the bus's transaction; the bus audits it after it returns (a failure to
record is logged, never reported as the run failing); its audit row
holds the handler's `result` like a `Tx` run's. It is the right
kind for exactly those commands, not a shortcut: `CommandBus::
external_commands()` is pinned by `the_external_commands_are_the_reviewed_ones`,
and adding one means naming its system in the summary. A
**`Dispatch`** handler (P7.A1) is `{ route, tx, external }`: after the
input passes the schema, `route(&input)` answers `Route::Tx` or
`Route::External(<system>)` (or the caller's `Invalid`), and the run
proceeds **exactly** as that kind — the same steps, one audit row under
the command's name. It exists for the `work_item.*` verbs, whose item may
be oxplow's (in the transaction) or another provider's (through its
process); `CommandBus::dispatch_commands()` is pinned like the External
list (`the_dispatch_commands_are_the_reviewed_ones`). Registering a
handler whose kind disagrees with the spec's `atomicity` is refused, as
is a second command of the same name.

## Commands so far

| Command | Handler | Notes |
|---|---|---|
| `<extension namespace>.<name>` (an enabled extension's `commands:`) | `Tx`: the extension's Starlark script composes core commands, run through `run_nested` (`extension_commands.rs`, P6b.B2) | declared invokers / confirm / effect, undoable, `Experimental`; registered while the extension is enabled (primary worktree). See [extensions.md](./extensions.md) → "Commands" |
| `command.sequence { calls: [{ name, input }] }` | `Tx` (`commands/compose.rs`, P6b.A1) | all invokers, `Write`, `Confirm::Never` — the children decide; undoable as the reversed children. Runs each call through `CommandBus::run_nested`: each child's own invokers, policy and confirmation; one audit row for the parent with the children in `result`; the one composition mechanism (an extension's command runs on it). See "Composition" |
| `work_item.transition { ref, to, native_state? }` | `Dispatch` (`commands/work_item.rs`, P2.6.3 / P7.A1): oxplow's items → `Tx` over `task_store::set_status_tx`; another provider's → its `transition` verb | all invokers, `Record`; undoable (the inverse restores the prior canonical and native state; an external inverse is renamed to `work_item.transition` so undo dispatches again). `to` is a canonical state, `native_state` the provider's own and must map to it (oxplow: its status; `archived` with `done` or `canceled`). For oxplow the row, the effort open/close, `work_item.transitioned` and `effort.*` commit with the audit, all caused by `command.executed`; the effort's snapshot pin is the effort-lifecycle pump consumer's. |
| `work_item.create { provider?, title, body?, parent_ref?, state?, native_state?, native? }` | `Dispatch` (tsk463 / P7.A1): oxplow → `Tx` over `task_store::insert_logged_tx` | all invokers; not undoable (that would be deleting an item). No `provider` files on the active one, which must be running. oxplow: `native { thread?, priority? }` (absent thread: the backlog); the row at the end of its list (`next_sort_index_tx`), `work_item.created@1 { work_item, status, effort? }`, and — filed `in_progress` on a thread — the effort, all caused by the run; an agent's task is authored `agent`. The result has the item's `ref`. |
| `work_item.update { ref, title?, body?, parent_ref?, state?, native_state?, native? }` | `Dispatch`: oxplow → `Tx` over `task_store::update_with_status_tx` | all invokers; undoable (the inverse restores exactly the fields and state given). oxplow: fields and status commit together — `work_item.edited@1 { work_item, fields }`, then the status move with everything `work_item.transition` implies; `native { priority? }` (a thread change is `work_item.move`). A refused run writes nothing. |
| `work_item.delete { ref }` | `Dispatch`: oxplow → `Tx` over `task_store::soft_delete_tx` (P6.E1b) | all invokers; `Destructive` (asks first); not undoable; only on a provider declaring `delete`. oxplow: marks the task deleted, closes its open effort, drops its body's `page_ref` edges and logs `work_item.deleted@1`, caused by the run. The UI's Delete (a right-click menu item, a page's inline confirm) is the confirmation. |
| `work_item.reorder { ref, before?, after? }` / `work_item.move { ref, to: "backlog" \| { thread }, before?, after? }` | `Tx` over `task_store::place_task_tx` (`commands/work_item.rs`, P6.E1a) | oxplow's lists only. All invokers; undoable — the inverse puts the item back next to the neighbour it had. `reorder` places an item before or after another of its own list (neither: at its end); `move` takes it to a thread's list or the backlog (at the end, or next to an item there), renumbering that list's `sort_index`. A move takes an `in_progress` task's claim with it (the open effort closes; one opens on the new thread — a claim, so only the writer thread may); logs `work_item.edited@1` (`thread` or `position`). An anchor from another list, both anchors, or an unknown thread is refused. |
| `work_item.link { ref, target, link_type }` / `work_item.comment { ref, body }` | `Dispatch`: oxplow → `Tx` over `task_satellite::create_link_tx` / `add_task_note_tx` (P5.C2) | all invokers; not undoable; only with the provider's `links` / `comments`; the target must be the same provider's. oxplow: a link type of its own list, made in the caller's thread (a person's: the linked task's, else the target's), or a note on the task, with its `page_ref` edges, logging `work_item.linked@1` / `work_item.commented@1` caused by the run. |
| `effort.open { work_item, thread? }` / `effort.close { effort, summary? }` | `Tx` over `effort_store::start_tx` / `finish_tx` (`commands/effort.rs`, P2.6.4) | all invokers; not undoable. For a work item whose provider doesn't open its own effort (`work_item:linear:ENG-12`): a registered provider declaring `in_progress_opens_effort` — oxplow's tasks, whose effort follows their status — is refused; an unregistered provider's item takes one. `thread` defaults to the caller's and must be its stream's working (active) thread; an agent may name only a thread in its own stream (and close only its stream's efforts). A second open on the same item is refused naming the open effort. Logs `effort.opened` / `effort.closed` caused by the run; the snapshot pin is the effort-lifecycle pump consumer's. |
| `config.list_keys {}` / `config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value, `human_only` |
| `config.set { key, value }` / `config.unset { key }` | `Tx`: validate against the key's schema, take the new document through the loader's own validation (`oxplow_config::keys::with_key`); after commit, write the file and swap the in-memory config | undoable (inverse restores the prior value or unsets); logs `config.changed@1 { key, before, after }`; `after_commit` broadcasts `ConfigChanged`; a **human-only key** (`HUMAN_ONLY_KEYS`: `ai`, `agents`, `agentModels`, `acpAgents`, `extensionInstances`, `lsp`, `collection`, `extensions`, `gauges`, `agentPromptAppend` — each runs a program, picks the model, enables code, or steers every agent; a test fails if a key documented as running programs or steering agents isn't listed) needs a person's confirmation per input |
| `metric.enable { keys, enabled }` | `Tx`: turns metrics on or off — computes the new `metrics:` list with `MetricsService::apply_metric_enabled` (a bundled gauge is off until a `use:` names it; a producer/plugin metric is on until an `enabled: false` marker) and hands it to `config.set`'s core; an unknown key (not in `metric_catalog`) is refused | undoable (restores the prior list); logs `config.changed@1 { key: metrics, … }`; the reseed follows `ConfigChanged` |
| `metric.record { key, value, subject?, dims?, stream? }` | `Tx` over `fact_store::record_facts_tx` (`commands/metric.rs`, P4.8) | not undoable. An asserted fact on the metric's source measure, stamped to match its filter, anchored to the stream's latest snapshot; the fact and the audit commit together. A formula or `count` metric, an unknown key, or another stream (for an agent) is refused. After commit: clears the fact memo, emits `MetricSamplesChanged` for the measure |
| `metric.run { key, stream? }` / `metric.rebuild { force }` | `External` over `MetricsService::run_metric_by_key` / `rebuild_baseline` | not undoable. Run one gauge now, or every gauge's whole-tree baseline. They drive snapshot captures and gauge scripts, which own their own transactions |
| `vcs.commit` / `vcs.stage` / `vcs.discard` / `vcs.fetch` / `vcs.pull` / `vcs.push` / `vcs.merge` / `vcs.checkout_branch` / `vcs.rename_branch` / `vcs.delete_branch` / `vcs.resolve_conflict`; `git.rebase` / `git.cherry_pick` / `git.revert` / `git.ignore` | `External` over the `Vcs` trait (`git.*`: the git provider's own ops) (`commands/vcs.rs`, P5.B6) | a person's only (`human`; agents run `git` in their terminal), not undoable. Each takes the `stream` it acts on, resolved strictly (an unknown stream is refused, never the primary's). `vcs.discard`, `vcs.merge`, `vcs.delete_branch`, `git.rebase` and `git.revert` are `Destructive` (confirmed). The result — and the audit row's — is the VCS's `OpOutcome { success, log, conflicts, auto_resolved }` (`vcs.commit`: `{ success, revision }`). After a run the stream's `WorkspaceChanged` (and `VcsRefsChanged`; every stream's after a fetch, push, rename or delete) is announced. See [vcs.md](./vcs.md) |
| `knowledge.write_page { slug, title?, body, verified_refs?, removed_refs? }` / `knowledge.delete_page { slug }` / `knowledge.link { page, target }` / `knowledge.resync { slug }` | `Tx` over `knowledge::write_page_tx` / `delete_page_tx` (`crates/oxplow-app/src/knowledge.rs`, P5.C3) | all invokers, `Record` (a read-only thread captures too); not undoable; `delete_page` is Destructive. Validates the slug and every `[[link]]` (refused, named), restates the `wiki_page` row and its pinned `page_ref` edges, logs `knowledge.page.written@1` / `knowledge.page.deleted@1` with the actor's anchors; writes (or removes) `.oxplow/wiki/<slug>.md` in the run; the UI re-reads on the row's `modelsChanged` (no wiki event of its own). See [knowledge.md](./knowledge.md) |
| `<provider>.<name>` (an enabled external provider's own declared commands — not its capability's verbs, which run as `work_item.<verb>`: the fake's `estimate`) | `External` over the provider process's `invoke` (`providers/registry.rs`, P5.D3) | registered while its instance is enabled (and removed when it stops), all invokers, `Experimental`; `confirm`, `effect` and `undoable` as the provider declares (its `inverse` becomes `<provider>.<command>`). The events its `invoke` returns are logged caused by the run — only types it declares, and a `work_item.recorded` only for its own items. See [providers.md](./providers.md) |
| `provider.enable { instance }` | `External` over the provider registry (`providers/registry.rs`, P5.D4) | a person's only, not undoable. Enables an extension provider's instance on this machine again — clearing an automatic disable — and reconciles, so it starts when `extensionInstances` enables it. Logs `provider.enabled@1 { instance }`; the result is the instance's view with its health. See [providers.md](./providers.md) |
| `lens.show { lens? , spec?, params? }` / `lens.keep { answer, extension?, slug? }` / `lens.share { lens, extension }` | `show`: `Tx`; `keep`, `share`: `External` (`commands/lens.rs`, P6.C1) | `show`: all invokers, `Record` — checks the spec's shape and its query through the read-only authorizer `query_sql` uses (`semantic_layer::check_query_on`), stores a `thread_answer` on the caller's thread (an agent may name only its own; a person any) with its open turn and effort, logs `lens.shown@1 { answer, thread, lens? }`; a refused query stores nothing. MCP `show_lens` runs it and returns the answer's text rendering (`text_answer`: rendered in the answer's own thread's worktree and lens context, the one resolution `run_answer` uses too). `keep`: all invokers, `Write`, not undoable — writes the answer as a private lens in `my-lenses` (its params become defaults, `intent.origin` the thread), sets `kept_lens`, logs `lens.kept@1`. `share`: a person's only — moves a lens into a shared extension (created with `sharing: shared` and `engine`), refused (and rolled back) when it doesn't load or its query reads anything but models. Both write lens files, which a `Tx` handler may not (the bus retries one on a busy database, and a retried file write strands the first): `keep` reads the answer in one transaction, writes the file, marks the row kept in another and removes the file when that fails; `share` must write before it can load-check. The bus records each run and its events after the handler returns. See [extensions.md](./extensions.md) → "Thread answers" |
| `metric.scaffold { key, title?, language?, glob? }` | `Tx`, `Read` | a starter gauge script and the measure + gauge + metric entries; writes nothing (the agent adds the entries with `config.set`) |

The `work_item.*` commands are **every** provider's
([work-items.md](./work-items.md)): each names its item by its canonical
ref and is dispatched to the ref's provider; an unregistered provider is
refused naming the registered ones (`no work-items provider \`linear\`;
registered: oxplow`).

**Callers.** Every task edit or status change made for someone is a
command, through `oxplow_app::task_writes` (which builds the commands'
input from oxplow's task shape — a status as oxplow's `native_state`,
thread and priority under `native` — and runs them through the
`WorkItems` client): `create` runs `work_item.create`, `update` runs
`work_item.update` (fields + status, atomic), `set_status` runs
`work_item.transition`, each settling the
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
`set_agent_model`, `set_generated`, `set_extension_enabled`)
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
invalid/denied/unknown are the caller's to fix, `Proposed` says the run
waits for a person (`run_command` returns it as a success instead),
`Failed` is internal.

## Exposure to agents (MCP)

Agents reach every command through two generic tools — `list_commands`
(the specs the calling agent may run, with `input_schema`, `summary`,
`confirm`, `undoable`) and `run_command { name, input }` (the outcome:
`result`, `audit_id`, `event_id`, `inverse?` — or, for a run that needs
a person's confirmation, `{ kind: "proposed", proposal, message }`). Extensions never add MCP
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
