# Commands: the one write path

What this doc covers: the command bus — what a command declares, who
may run it, the pipeline every run goes through, the audit and undo,
and how a new command is added. Built in
P1.8–P1.10 (tsk410–412): the bus, `config.*`, and the generic MCP
surface (`list_commands` / `run_command`) with caller identity.

## What a command is

A command is a typed operation identified by its **command id**,
`<namespace>.<area>.<verb>` in snake_case (`oxplow.work_item.transition`,
`oxplow.config.set`, `acme_pr.issue.close`; `CommandSpec::validate_id`).
The id is the machine name; what a person reads is the command's label.
The **namespace** is its owner's, so two extensions' areas never collide:

- `oxplow` — **reserved** for oxplow's own commands: core's, and those of
  the extensions that ship with oxplow (`oxplow-bundled` declares
  `namespace: oxplow`). They share it, collisions checked per id, so a
  command moving between core and a shipped extension keeps its id.
- An extension's declared `namespace:` (default: its name with `-` →
  `_`) — that extension's alone.
- A provider instance's id — interim, `<instance>.<capability>.<name>`
  (`providers::command_id`), until providers are namespaced by their
  extension.

The registry records each command's **source** (`core`,
`extension:<name>`, `provider:<name>`; `CommandBus::source_of`) and each
namespace's **holder** (`namespace_owner`: `oxplow`, or the one source
holding it). `register` is core's (under `oxplow`); `register_namespace(ns,
source, commands)` registers a source's commands all-or-nothing;
`unregister_source` removes one source's (an extension disabled, a
provider stopped), freeing a namespace left empty. Migration V26 rewrote
history (audit, proposals, `command.*` payloads) from the old two-part
ids. A command declares (`oxplow_domain::commands::CommandSpec`):

| Field | Meaning |
|---|---|
| `input_schema` | JSON Schema; the bus validates the input first and names the failing field, plus what the schema accepts there (`InputValidator::check`: an object's fields, required first, for an unknown or missing one; the choices for a bad enum value), so a caller fixes it in one more call |
| `invokers` | which surfaces may run it: `human`, `agent`, `lens` |
| `confirm` | `Never`, `Always`, or `Destructive` — a person confirms; an agent never can. A `Read` command may not ask (`Command::new` / `with_confirm_for` refuse it — so a read capability's command can't declare `confirm`): it runs unrecorded, so nothing would resolve its proposal |
| `undoable` | the handler returns an inverse call that `undo` applies |
| `lifecycle` | `Stable` / `Experimental` |
| `atomicity` | `Tx` (handler runs inside the bus's transaction), `External` (against a system the bus doesn't own — a VCS, a process, a work list), or `Dispatch` — a composite, which its calls decide ("Composition") |
| `effect` | `Write` (the default), `Read` or `Record`. A read runs without an audit row or `command.executed`, so a polling agent doesn't fill the log, and a thread that may not write can still run it (`oxplow.config.list_keys`, `oxplow.config.get`). A `Write` is refused to an agent thread that may not write. A `Record` changes oxplow's own records (`work_item.*`, `effort.*`): audited like a write, open to any thread |
| `needs` | the **host capabilities** its handler calls (`sql.read`; below — always available, enforced per call) and the capabilities, or their features (`work_items.comments`), it needs active — the grammar lenses use. Unmet, it isn't offered and doesn't run (pipeline step 0). `oxplow.work_item.create` / `update` / `transition` need `work_items`; `link`, `comment` and `delete` need their feature |

`Actor` is who runs it: `Human`, `Agent { thread_id, stream_id }`,
`Lens { lens_id, on_behalf_of }`, `System`. Its `source()` (`human`,
`agent:thr3`, `lens:acme/x`) is what the event and audit record.

## Offering a command to a person

A command says how a person meets it with an optional `ui`
(`oxplow_domain::CommandUi`; agents ignore it; a manifest command's `ui:` — oxplow's own in `extensions/oxplow-foundation`):

| Field | Meaning |
|---|---|
| `label` | what a person reads (`Pull Changes`, `New Task…`) |
| `group` | what search lists it under (`Git`, `Tasks`) |
| `keywords` | more words search matches it by |
| `about` | the ref kind it acts on: offered on that ref's page and rows; absent, it needs no ref and **search** offers it |
| `input` | the input it runs with; a string exactly `{{stream}}`, `{{thread}}`, `{{ref}}` or `{{ref.id}}` binds from where it runs, and one with nothing to bind there makes it unavailable there |
| `form` | the page that gathers its input (`page:new-task`): choosing it opens that page instead of running it |
| `open_after` | a tab id to open once it ran, `{{result.<field>}}` from its result (`page:custom-dashboard?id={{result.id}}`) |
| `background` | it runs as a background task (kind `vcs` for `oxplow.vcs.*`, else `command`), its failure an op error |

RPC `list_person_commands` lists what a person is offered: what they may
run now (invokers, `needs` active) that has a `ui`. The desktop keeps one
listing (`personCommandsStore.ts`, reloaded when extensions or config
change) and turns the ref-less ones into search entries with
`commandOffers.ts` (pure: binding, availability, run / form / background /
open-after); a shortcut may run one by id (⌘⇧N → `oxplow.work_item.create`'s
form). Offered now: `oxplow.vcs.pull` / `push`, `oxplow.work_item.create`,
`oxplow.dashboard.create`, `oxplow.stream.create_worktree`. Commit, New
Thread and New Lens with Your Agent are still the app's own commands
(`commands.ts`) until the window provides them as capabilities.

## Host capabilities

What a handler may do beyond composing other commands is a **host
capability**: named like an OAuth scope, `<resource>.<action>`, declared
by core with an **effect class** (`oxplow_domain::host_capability`:
`HOST_CAPABILITIES`, `EffectClass`), and listed in the command's
`needs`. Every command — core's and any extension's — reaches them the
same way, so oxplow's own commands can do nothing an extension's can't.

| Class | Does | In the run's transaction | Audited |
|---|---|---|---|
| `view` | changes only what the person sees (open a page) | no | no |
| `read` | reads (`sql.read`) | yes | no |
| `record` | changes oxplow's own records | yes | yes |
| `write` | changes files, git, processes | no | yes |

A command's effect is the strongest class it needs (`strongest_class`).
Today there is one: **`sql.read`** — `{ sql, params? }`, one read-only
statement over the published models (`v_*`), its `:name`s bound from
`params`, at most `SQL_READ_ROW_CAP` (1000) rows, answered as a list of
row objects (`semantic_layer::read_on` on the run's connection: the
`query_sql` authorizer, inside the transaction).

A Starlark handler calls one with `capability(id, args)`
(`oxplow_collect_plugin::capability`). The script runs on the sandbox's
worker thread; the call crosses to the thread that runs the handler
(which owns the transaction) and is answered there while the sandbox
waits (`run_starlark_serving`) — that time is left out of the script's
budget, not its ceiling. `crate::host_capabilities::Calls` serves it: a
capability that isn't in the command's `needs` is **refused** (the
script fails with the reason), one that is is counted and answered; a
busy database retries the whole run. A dry run (an extension's
`examples`) answers from the example's `answers` (per capability, in
call order) or through the SQL gateway.

**Operations** (`commands/ops.rs`). A record or write capability's
native behavior comes as **operations**: `bookmarks.write` has `set` and
`remove`. Each `Op` carries what only Rust can supply — the input schema
of the type its handler reads, the handler (`Tx` / `External` /
`Dispatch`), whether it returns an inverse, a per-input confirmation, a
check before the transaction — and is added to the bus (`add_op`). A
manifest command backed by one (`capability:` + `op:`) gets the
operation's schema, undo and atomicity, its effect from the capability's
class (record → `Record`, write → `Write`, read / view → `Read`), and
the capability in its `needs`; the manifest says the rest. oxplow's own
are declared in **`oxplow-foundation`** — a shipped extension that is
**required** (`BundledExtension::required`: it can't be disabled, and
`extension_commands::register_required` registers its commands when
services are built, before anything runs one; the reconciler leaves it
alone). An extension may declare a command over the same operation in
its own namespace. A script reaches record and write operations only by
composing commands.

**The per-run trace.** Each run has a `CapabilityTrace` (`TxCtx::trace`):
fresh per transaction attempt, shared by the runs nested in it; steps
add each step's to the composing pass's. Its summary — per capability,
how many calls (`{"sql.read": 2}`) — is written on the run's audit row
(`command_audit.capabilities_json`, V27; NULL when it used none). The
routing pass (a composite composed on a read snapshot to decide where
its calls run) counts nothing. Later: each call's detail (what it
touched), sampled or on by setting, and approvals by capability.

## The pipeline (`crates/oxplow-app/src/commands/mod.rs`)

`CommandBus::run(actor, name, input, confirmed)`:

0. **Offered** — what's active now (`with_capabilities`: the capability
   registry and the config, `capabilities::Active::refusal`) must meet the
   spec's `needs`, and a command an implementation owns runs only while
   that one is active: a provider instance's namespace (`<instance>.*`).
   Refused
   → `Invalid` naming what it needs (audited), before validation, for
   every actor; `list` leaves it out the same way.
1. **Validate** the input against the schema → `Invalid { field, message }`
   (audited as `invalid`).
2. **Invoker check** — the spec's `invokers` must admit the actor's surface
   → `Denied` (audited).
3. **Agent policy** — for an agent-driven run (an agent, or a lens
   acting for one, nested runs included), `AgentPolicy::check_command(
   thread, spec, may_write)` (`agent_policy.rs`) → `Denied` (audited).
   A spec with `invokers.agent: false` is denied here — so a lens acting
   for an agent can't run what the agent can't, and gets no proposal
   (tsk729): this is the one place that rule lives. A `Write`
   command needs a thread that may write: the bus asks its `WriteGate`
   (`Services` wires "the thread exists and is its stream's writer"), so
   a queued or closed thread can read but not change state (tsk437
   review). A `Record` command skips that refusal: oxplow's own records
   (tasks, efforts) aren't a claim on the worktree.
   **Pre-check** (tsk1010): a command may carry a
   `Precheck` (`Command::with_precheck`) — an async check of the input
   that needs more than a transaction can do (resolving a metric query
   through the metric engine). It runs here, once the actor is admitted
   and before any transaction: the command's own for a direct call; for a
   composite, every composed call's, collected while routing
   (`steps.rs` `Router`), its refusal placed at `/calls/<i>/input…`. A
   refusal is audited `invalid` and nothing runs.
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
   merge's conflicts, stays readable after the fact — inverse, and the
   host capabilities it called, "Host capabilities"), `command.executed@2` in the event log pointing at
   the audit row, the handler's domain events (with `cause` = the
   executed event, and the actor's thread/stream — `Actor::anchors()` —
   filled into any anchor the handler left empty), and the audit row's
   `event_id`. A handler failure
   rolls all of it back and is audited as `error` in a transaction of its
   own. A handler that found **nothing to change** and wrote nothing
   returns `HandlerOutput::unchanged` (tsk861): a plain call then leaves
   no record — no audit row, no `command.executed`, `audit_id: None` —
   like a read (`oxplow.knowledge.relocate_comment` re-run on a settled
   document is the case). The bus enforces it (tsk901): an unchanged call's
   transaction is **rolled back**, so nothing it wrote lands unaudited,
   and one that also returns events or an inverse is refused (audited as
   `error`). An undo, an approval or an effect's reaction is recorded
   regardless: its row must be marked (`an_undo_or_approval_that_changes_
   nothing_is_still_recorded`). Only `Tx` handlers may say it; an
   `External` run is always recorded.
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

**Only a person confirms; an effect runs with an agent's rights**
(P8.D8). `Actor::may_confirm()` is true for `Human` and a lens acting for
one — never an agent, an effect or `System` (step 4 drops a `confirmed`
from anyone else). `Actor::proposes()` — an agent's or an
`Actor::Effect { effect: "<extension>/<id>" }`'s run, directly or through
a lens — turns a run that asks into a proposal; `System` is asked
(`NeedsConfirmation`). An effect's invoker is `Agent` (a person-only
command is `Denied`), it has no thread (no write gate, nothing
thread-scoped), its source is `effect:<extension>/<id>`, and only a
`Human` approves or declines a proposal. The audit and events say so:
`actor_kind = 'effect'` (V147 rebuilds `command_audit`'s CHECK, keeping
proposals' `audit_id`), carried by `command.executed@2` and
`command.proposed@2` — v1 plus `effect` in `ActorKind`, upcast as is; the
v1 schemas keep the frozen `ActorKindV1`. An effect's reaction runs
through `CommandBus::run_effect` (P8.D10) with `RunOrigin::Effect`, the
fourth origin beside a call, an undo and an approval: like those, its
record (`effect_run` + `effect.result@2`) lands in the run's transaction,
or is claimed before a step outside it (`claim`), and its
`command.executed` is caused by the triggering event (extensions.md
"Effects").

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
tolerate failure since the run is already recorded. `oxplow.config.set` is the
model: the handler validates and computes before/after; `after_commit`
re-applies the key to the config as it is then, writes project.yaml and
swaps memory under one lock, so concurrent sets of different keys both
survive.

## Composition: `oxplow.command.sequence`, `run_nested` and steps (P6b.A1, tsk713)

A composite is a command made of other commands' calls: a
`Handler::Compose` (`commands/compose.rs`) whose **composer** says, for
an input, which calls to run (`Composition { calls, result, events }` —
`events` are the composite's own, appended after the children's and
caused by its `command.executed`; on the steps path only when every step
landed; an extension command's declared types, P8.D4). It is the
one mechanism: `oxplow.command.sequence { calls: [{ name, input }] }` composes
its input's calls; an extension's own command (P6b.B2) composes what its
script returns over its `input` rows. Its atomicity is `Dispatch` — its
calls decide where it runs, so composites are pinned
(`CommandBus::composite_commands`, `the_composite_commands_are_the_reviewed_ones`).
Once the actor is admitted (steps 2–3), the bus **routes** it
(`CommandBus::route_composite`, `commands/steps.rs`): it composes on a
read snapshot and routes each call — a `Tx` command stays in the
transaction; an `External` one (every work list's `work_item.*` verbs,
oxplow's own tasks' included) leaves it; a nested composite must route inside (its
steps would otherwise land outside the run they're a step of), and a
`Read` call is refused (a composite composes commands that write).
Every call inside → **one run in one transaction**, below; any outside →
**steps** (next section).

**In one transaction**: `CommandBus::run_nested(ctx, parent_spec,
calls)` (`commands/mod.rs`), the composite's `Tx` handler — composing
again, in the run's own transaction. It first makes a pass that writes
nothing: every call must join the transaction (a call that leaves it is
refused, naming the system and the composite), its input must fit (a problem
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
`result` is `{ result, children: [{ name, result }] }` — no child's
`input` (the caller sent it) or `inverse` (the parent's is the undo):
echoing both cost an agent filing twenty tasks 23k characters back;
the children's events ride out on the parent's `HandlerOutput.events`,
so they are caused by the parent's `command.executed`; their
`after_commit`s chain in order. The inverse is the children's inverses,
**reversed**, as a `oxplow.command.sequence` — or none when a child has none —
so `undo` needs nothing new, and a child whose inverse asks makes the
undo ask. A child's `Busy` propagates and the bus retries the whole
parent (handlers are pure).

**As steps** (P7 review, tsk713; `CommandBus::run_steps`): a composite
with a call outside the transaction can't be all-or-nothing, so its
calls run as steps. (1) **Checked first**: every step's input (at
`/calls/<i>/input/…`), its own invokers and the agent policy, before
anything runs — a refusal is the run's, audited, and nothing ran; a step
that asks makes the run ask once (`NeedsConfirmation` with the calls in
the preview), and an agent's run is a proposal **with no dry run**
(nothing outside the transaction runs before a person decides). (2)
**In order**, each landing as it runs: a step inside oxplow in its own
transaction (its events caused by the run's `command.executed`, whose id
is fixed first), an external one through its system; the first failure
stops the run, and the steps before it **stand**. (3) **Recorded once**:
one audit row and one `command.executed` with `result { result,
children: [the landed steps], failed?: { name, input, error } }` and the
landed steps' events, caused by it. A run that stopped partway is
recorded as an `error` with what landed, and returns `Failed` naming the
failed step and the ones that stand; a run that failed at its first
step is audited like any failure. **Not undoable** (no inverse): a
system outside the bus can't be rolled back with it, so a person
reverses a landed step by hand (each child keeps its own `inverse` in
the result).

## Proposals: an agent's run that needs a person (P6b.A3)

A run an agent can't confirm is kept for a person instead of being
refused (`CommandBus::unconfirmed`):

1. **Dry run** — a `Tx` handler runs with `confirmed: true` in a write
   transaction that is always rolled back (`Database::rehearse`); its
   `result` is what it would have done (`oxplow.config.set`: `{ key, before,
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
   result `{ kind: "proposed", proposal, message }` (`proposed_message`: it waits
   for the person in this thread and in Alerts, and on the setting's
   row; "It replaces proposal:M" when it did; tell the person; don't run
   it again; its `decision` is in `v_command_proposal`). Other MCP
   tools that run a command report the same message as an error. The
   desktop shows a thread's pending proposals where the conversation is
   (P9.A3: the Answers strip, and under the proposing call in an ACP
   transcript — `useThreadProposals`, `proposalOfTool`; usability.md
   "Agent proposals").

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
schema. A `oxplow.config.set` / `oxplow.config.unset` writes **only its key** into the
file (`write_project_key`, tsk1028): that key's block — its line at column
0 through the next key, under its own comments — is replaced, removed or
appended, and every other line stays byte for byte as the person wrote it
(flow style, comments, keys equal to their default such as
`agents: [claude]`). The edit is checked to change that key and nothing
else; a file whose YAML ties keys together (anchors) is refused, to be
edited by hand. The whole-file re-render it replaced lost comments and
formatting and dropped default-valued keys. Adding a field
to `RawConfig` makes it managed, documented and settable at once.

**A person's layer.** `oxplow.config.set` / `oxplow.config.unset` take `layer: project`
(the default) or `layer: personal`: `.oxplow/personal.yaml`, a person's
own layer over the project's, which git ignores (`.oxplow/.gitignore`
keeps only `project.yaml`). It holds only `oxplow_config::PERSONAL_KEYS`
(`activeProviders`, a person's choice of implementations); another key
there is refused, by the command and by the loader. A personal key keeps
its key's human-only rule. `config.changed@2` names the `layer`; `@1`
payloads were all the project's (V14 moved them). `effective_config`
shows the layer as its own `personal.<key>` row, origin `personal`.
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
a person-only key's `oxplow.config.set` by the agent becomes a proposal the
person approves. Direct
controls remain only for person-only settings (agents, AI, language
servers, extensions, integrations, programs); `set_snapshot_retention_days`
and `set_snapshot_max_file_bytes` went with their editors. The view
re-reads on `ConfigChanged`, the renderer's one signal for every config
write: a `oxplow.config.set`, and `set_ai_role` (a role's binding is an
`ai.roles.<role>` row), which emits it too (P6 review, tsk607).

**The backend reacts on the pump, never the bus** (P7.B6,
`config_reactors.rs`): `config.changed@2` consumers, each filtered to its
keys — `config.workspace_filter` (`generated`), `config.extensions`
(`extensions`: the extension catalog's change signal),
`config.providers` (`extensionInstances`, `activeProviders`: the provider
registry reconciles) and `config.metrics` (any key: the metric catalog
reseeds). One that reads the whole config first waits for
`oxplow.config.set`'s after-commit swap (`applied`: the key no longer reads its
`before`, woken by `Services.config_applied`, at most 5 s), since the
pump can see the commit before the swap.

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent, the launcher for the human.

## `Tx` and `External`

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
retries reaches the caller, as `Busy` (RPC `BUSY`). A system outside
oxplow that couldn't answer just now (a provider's `Internal`, a timeout,
a rate limit, with its wait) is `Unavailable { message, retry_after_ms }`
(RPC `UNAVAILABLE`) — the one failure an effect sends again by itself
(tsk914); everything else that fails is `Failed`. An **`External`**
handler (P5.A1) is an async call against a system the bus doesn't own —
a VCS, a provider process, a collector script, the lens files on disk —
whose state can't join the bus's transaction; the bus audits it after it returns (a failure to
record is logged, never reported as the run failing); its audit row
holds the handler's `result` like a `Tx` run's. It is
`Fn(Invocation, Value)`: `Invocation { actor, idempotency_key }`, the key
set for a step of an effect's reaction (`effect_step_key`:
`effect:<effect>:<event id>:<index>:<hash of the call>`, the same on
every attempt at it — P10) and `None` otherwise; a provider's handlers
pass it to the provider ([providers.md](./providers.md)
"Idempotency"). A keyed step's events take dedupe keys from it
(`<key>:event:<i>`, `same_events_once`) and `record_tx` appends handler
events uniquely, so a step that landed and is re-sent on a retry — its
system answering again, events included — logs them once (tsk912). It is the right
kind for exactly those commands, not a shortcut: `CommandBus::
external_commands()` is pinned by `the_external_commands_are_the_reviewed_ones`,
and adding one means naming its system in the summary. The
`work_item.*` verbs are `External` for every list, oxplow's own tasks
included: each runs on the item's list through its `WorkItemVerbs`
([work-items.md](./work-items.md)). Registering a handler whose kind
disagrees with the spec's `atomicity` is refused, as is a second command
of the same name.

**A run's projections are delivered before it returns.** Once a run
lands (one transaction, an outside call recorded, or a composite's
steps), `CommandBus::landed` runs the event pump's sync consumers — the
projections: a work list's records into `work_item`, the page refs —
before the caller gets the outcome (`EventPump::deliver_projections`),
then wakes the pump's loops. So whoever ran a write reads it back at
once, whichever list holds it.

## Commands so far

Each `oxplow.*` row below is declared in `extensions/oxplow-foundation`
(or, where it says so, `oxplow-bundled`) over an operation of the host
capability its area falls under (`threads.write`, `vcs.write`, …;
`HOST_CAPABILITIES`); "Handler" describes that operation.

| Command | Handler | Notes |
|---|---|---|
| `<extension namespace>.<name>` (an enabled extension's `commands:`) | `Tx`: the extension's Starlark script composes core commands, run through `run_nested` (`extension_commands.rs`, P6b.B2) | declared invokers / confirm / effect, undoable, `Experimental`; registered while the extension is enabled (primary worktree). See [extensions.md](./extensions.md) → "Commands" |
| `oxplow.review.accept`, `oxplow.review.request_changes` (oxplow-bundled, P7.C5) | extension composites of `oxplow.work_item.comment` + `oxplow.work_item.transition` (to `done` / `todo`) on an effort's work item — as steps, every list's verbs running outside the transaction (not undoable); an effort without a work item is refused by name; accept refuses unreviewed claims or inferred decisions unless `force` | human + lens, not agent; `confirm: always`. See [extensions.md](./extensions.md) → "oxplow-bundled" |
| `oxplow.command.sequence { calls: [{ name, input }] }` | `Dispatch` — a composite (`commands/compose.rs`, P6b.A1) | all invokers, `Write`, `Confirm::Never` — the children decide. Each child's own invokers, policy and confirmation; one audit row for the parent with the children in `result`. Every call in oxplow's records → one transaction (`run_nested`), undoable as the reversed children; a call outside it (a `work_item.*` verb, any list's) → steps, in order, not undoable. The one composition mechanism (an extension's command runs on it). See "Composition" |
| `oxplow.work_item.transition { ref, to, native_state? }` | `External` (`commands/work_item.rs`): the item's list's `transition` verb (oxplow's tasks: `oxplow_tasks::verbs::transition_tx`, in a transaction of their own) | all invokers, `Record`; undoable (the inverse restores the prior canonical and native state; an external inverse is renamed to `oxplow.work_item.transition` so undo dispatches again). `to` is a canonical state, `native_state` the provider's own and must map to it (oxplow: its status; `archived` with `done` or `canceled`). For every provider core logs `work_item.state_changed` when the state moved, with the audit, caused by `command.executed` (the interface's events: [work-items.md](./work-items.md)), which the effort policy reacts to ([work-tracking.md](./work-tracking.md)). |
| `oxplow.work_item.create { title, body?, parent_ref?, state?, native_state?, native?, thread? }` | `External`: the active list's `create` (oxplow's tasks: `oxplow_tasks::verbs::create_tx`) | all invokers; not undoable (that would be deleting an item). Always files on the active tracker, which must be running. `thread`: absent, an agent's own; a person's without one, the backlog. oxplow: `native { priority? }`; the row at the end of its list (`next_sort_index_tx`), core's `work_item.created@2 { work_item, state }` and `work_item.state_changed`, caused by the run; an agent's task is authored `agent`. The result has the item's `ref` and `link_warnings`: the body's `[[…]]` links that don't resolve, checked like a note's, for every list (`link_check::LinkDeps::item_warnings`, on a read of its own). |
| `oxplow.work_item.update { ref, title?, body?, parent_ref?, state?, native_state?, native? }` | `External`: the item's list's `update` (oxplow's tasks: `oxplow_tasks::verbs::update_tx`) | all invokers; undoable (the inverse restores exactly the fields and state given). oxplow: fields and status commit together — core logs `work_item.edited@2 { work_item, fields }` for the fields the input set, then the status move with everything `oxplow.work_item.transition` implies; `native { priority? }` (a thread change is `oxplow.work_item.move`). A refused run writes nothing. An update that sets a body carries its `link_warnings`, as `oxplow.work_item.create`'s does. |
| `oxplow.work_item.delete { ref }` | `External`: the item's list's `delete` (oxplow's tasks: `oxplow_tasks::verbs::delete_tx`) | all invokers; `Destructive` (asks first); not undoable; only on a provider declaring `delete`. oxplow: marks the task deleted; its record says so, which clears its row and its page refs; core logs `work_item.deleted@2`, caused by the run. The UI's Delete (a right-click menu item, a page's inline confirm) is the confirmation. |
| `oxplow.work_item.reorder { ref, before?, after? }` / `oxplow.work_item.move { ref, to: "backlog" \| { thread }, before?, after? }` | `External`: the item's list's `reorder` / `move` (oxplow's tasks: `oxplow_tasks::verbs::reorder_tx` / `move_tx`) | Only on a list declaring `ordering` / `lists`. All invokers; undoable — the inverse puts the item back next to the neighbour it had. `reorder` places an item before or after another of its own list (neither: at its end); `move` takes it to a thread's list or the backlog (at the end, or next to an item there), renumbering that list's `sort_index`. Logs core's `work_item.edited@2` (`list` and `rank`, or `rank`). An anchor from another list, both anchors, or an unknown thread is refused. |
| `oxplow.work_item.link { ref, target, link_type }` / `oxplow.work_item.comment { ref, body }` | `External`: the item's list's `link` / `comment` (oxplow's tasks: `oxplow_tasks::verbs::link_tx` / `comment_tx`) | all invokers; not undoable; only with the provider's `links` / `comments`; the target must be the same provider's. oxplow: a link type of its own list, made in the caller's thread (a person's: the linked task's, else the target's), or a note on the task (its page refs restated from the interface), logging core's `work_item.linked@2` / `work_item.commented@2` caused by the run. |
| `oxplow.effort.open { thread?, work_item?, title?, adopt_since? }` / `oxplow.effort.close { effort \| thread, as_of?, summary?, reason? }` / `oxplow.effort.link { effort \| thread, work_item }` / `oxplow.effort.update { effort, title }` | `Tx` over `effort_store::start_tx` / `finish_tx` / `link_tx` / `retitle_tx` (`commands/effort.rs`) | all invokers, `Record`: an effort is a record, not a claim on the worktree, so any thread may run them (an agent only in its own stream). `open` needs no work item; the thread's open effort closes first (`switch`), and `adopt_since` adopts the thread's un-efforted activity back to there, never before its previous effort ended. `close` names the effort or the thread's open one; `as_of` closes it at a past point and releases what came after; `reason` (`commit` / `switch`) records why, else it's the caller's (`agent`, `person`, an effect's `system`). `link` (`null` unlinks) and `update` (`null` clears the title) undo to what was there. See [work-tracking.md](./work-tracking.md) |
| `oxplow.effort.record_decision { thread?, work_item?, question, choice, alternatives?, confidence?, why? }`, `oxplow.effort.record_claim { thread?, work_item?, statement, kind, evidence_ref? }` | `Tx` over `reasoning_store::{record_decision_tx, record_claim_tx}` (`commands/reasoning.rs`, P8.A7) | all invokers, `Record`, `Confirm::Never`, not undoable. An agent's land on its own thread whatever thread it names (another is refused); a person names one. They attach to `work_item`'s open effort — refused when the item is another stream's work — else the thread's newest open effort. Read back as `v_decision` / `v_claim` |
| `oxplow.effort.report { thread?, summary?, impacts? }` | `External` (`commands/effort_report.rs`): it checks the summary's links against the worktree | all invokers, `Record`, not undoable; optional. An agent reports only on its own thread; a person names one. It settles the effort lifecycle first, then records the summary and impacts on the thread's open effort, else its latest (refused at `/thread` when it has none) — never opening, closing or creating one — and returns `{ effort, link_warnings }`. Files and runs are observed, never reported, and without a summary `v_effort.summary` is the effort's last turn's final message ([work-tracking.md](./work-tracking.md)) |
| `oxplow.test.record_run { thread?, command, duration_ms?, passed?, failed?, total? }` | `External` over the `CollectionService` (`commands/test_runs.rs`): the capture commits in the collector's own transaction | all invokers, `Record`, not undoable. An agent's goes on its own thread whatever it names; a person names one. The `asserted` run whose counts oxplow couldn't parse; it is the thread's open effort's. A report oxplow parses itself is a report collector's, run by hand with `oxplow.collector.sync`. See [collection.md](./collection.md) |
| `oxplow.extension.install { git_url, git_ref?, reviewed_sha, stream? }`, `oxplow.extension.update { name, reviewed_sha, stream? }` | `External` (`commands/extension_install.rs`, P8.A9): a clone into `.oxplow/tmp/` and a copy into a stream's worktree | all invokers, `Write`, `Confirm::Always`, not undoable. What an extension can run is a person's call, against the review of the commit they saw (`reviewed_sha` is the only commit installed): an agent's run becomes a proposal; Settings → Extensions runs it confirmed. `stream` names the worktree that takes it (the app shows it once that stream is merged); absent, the primary checkout — never the actor's own, since the handler only ever runs as the person who confirmed (an agent's run is a proposal, which shows the input as it will run; tsk786). See [extensions.md](./extensions.md) |
| `oxplow.snapshot.restore_file { file_snapshot }` | `External` over `snapshot_files::SnapshotFiles` (`commands/snapshot.rs`, P8.A9) | all invokers, `Write`, `Confirm::Destructive` (it overwrites the file at its path), not undoable: a person confirms, an agent's run is a proposal. Writes into the row's stream's worktree |
| `oxplow.lsp.install_server { package }`, `oxplow.lsp.remove_server { package }` | `External` over the `LspInstallerService` (`commands/lsp.rs`, P8.A9): the network and `.oxplow/lsp/` | all invokers, `Write`, `Confirm::Always`, not undoable: what binaries oxplow downloads and runs is a person's call. Each pushes `LspServersChanged` (the server list isn't a model). See [lsp.md](./lsp.md) |
| `oxplow.thread.create { stream, title, agent?, acp_agent?, from? }`, `oxplow.thread.rename { thread, title }`, `oxplow.thread.set_prompt { thread, prompt? }`, `oxplow.thread.promote { thread }`, `oxplow.thread.demote { thread }`, `oxplow.thread.close { thread }`, `oxplow.thread.reopen { thread }`, `oxplow.thread.reorder { stream, order }` | `Tx` over `thread_store::{get,list_for_stream,upsert}_tx` (`commands/thread.rs`, P8.A3) | `Record`, `Confirm::Never`. The thread state machine: a new thread is the stream's writer when it has none, else queued at the end (`from: thread:<id>` forks it with the source's agent and ACP agent); `promote` demotes the writer in the same run (undo promotes it back — or, when the stream had no writer, demotes this one); `demote` moves the writer back into the queue, leaving none (undo promotes it); `close` ↔ `reopen` undo each other (a close also closes the thread's open effort, `closed_by` `system`, in the same transaction; a reopen leaves it closed), and closing an ACP thread stops its session after commit; `rename` / `set_prompt` / `reorder` undo to what they were; `create` isn't undoable. Threads and streams are named by ref (`thread:thr12`, `stream:str1`). An agent acts only on its own stream and closes and reopens only its own thread; `set_prompt`, `promote`, `demote` and `reorder` are a person's (or a lens's) — a prompt and the writer steer an agent. The desktop's thread helpers run these through `run_command`; the views re-read on `ModelsChanged` naming `v_thread` |
| `oxplow.stream.create_worktree { slug, title, branch, branch_source }`, `oxplow.stream.adopt_worktree { path, title }`, `oxplow.stream.archive { stream, delete_worktree? }` | `External` over `StreamService` (`commands/stream.rs`, P8.A4) | a person's (`HUMAN_ONLY`), `Write`, not undoable. Create and adopt reach the VCS and start the stream's capture service; archive is `Confirm::Destructive` (the desktop's Remove dialog is the confirmation), refused while an agent runs in one of the stream's threads, closes each of its threads' open efforts (`closed_by` `system`) at an `effort_end` snapshot taken first — before the working copy can go (`SqliteEffortStore::close`) — takes its threads with it, purges its search rows, forgets its worktree route and stops its capture, and removes its working copy when `delete_worktree`. A worktree that vanishes is archived by the workspace watcher directly — an observation, not an intent |
| `oxplow.stream.rename { stream, title }`, `oxplow.stream.set_prompt { stream, prompt? }` | `Tx` over `stream_store::{get,upsert}_tx` (P8.A4) | `Record`, undoable to what they were. An agent renames only its own stream; the prompt (appended to every agent prompt in the stream) is a person's or a lens's. The stream list re-reads on `ModelsChanged` naming `v_stream` |
| `oxplow.hint.dismiss` (`Tx`) | `commands/hint.rs` over `agent_nudge_store::dismiss_tx` | `Record`; a person's or a lens's, never an agent's: dismisses a hint raised to the person (`v_agent_nudge`, audience `person`), stamping its `delivered_at`. Not undoable |
| `oxplow.bookmark.set`, `oxplow.bookmark.remove` (`Tx`) | `commands/bookmark.rs` over `bookmark_store::*_tx` | `Record`; a person's or a lens's, never an agent's (bookmarks are the person's navigation). The viewer is `thread` (its stream implied) or `stream`; `set` moves a page's bookmark to its `scope`. Both undo to what the viewer saw before |
| `oxplow.dashboard.create`, `oxplow.dashboard.rename`, `oxplow.dashboard.delete`, `oxplow.dashboard.remove_item`, `oxplow.dashboard.reorder_items` (`Tx`); `oxplow.dashboard.add_item`, `oxplow.dashboard.update_item` (`External`) | `commands/dashboard.rs` over `dashboard_store::*_tx` (P8.A5) | `Record`; all invokers except `delete` (a person's or a lens's, `Confirm::Destructive`, not undoable). `create` isn't undoable; the rest undo (a removed tile comes back at its position). Adding or editing a tile is `External` because its SQL is checked by the semantic engine (async) first. See [dashboards.md](./dashboards.md) |
| `oxplow.knowledge.add_comment { stream, thread?, target, quote, selectors_json, context_chain?, referenced_refs?, intent, body }`, `oxplow.knowledge.reply_comment { comment, body }`, `oxplow.knowledge.update_comment { comment, intent?, status?, quote? + selectors_json? }`, `oxplow.knowledge.delete_comment { comment }` | `Tx` over `comment_store::*_tx` (`commands/comment.rs`, P8.A6) | `Record`, logging the existing `knowledge.comment.*` events caused by the run. The author is the actor (`user` for a person, `agent`, the person a lens acts for) — never an input. An agent comments only in its own stream and thread. `update_comment` undoes to the intent/status/quote it replaced (a relink is `quote` + `selectors_json`); `add`/`reply` aren't undoable; `delete` is a person's or a lens's, `Confirm::Destructive`. `oxplow.knowledge.relocate_comment { comment, selectors_json, orphaned }` (tsk861) is the renderer's re-anchor after it re-finds the quote — a person's or a lens's, not undoable; one already where it was is `unchanged` and leaves no record. Views re-read on `ModelsChanged` naming `v_comment` |
| `oxplow.knowledge.add_note { thread?, body }`, `oxplow.knowledge.update_note { note, body }` | `Tx` over `oxplow_tasks::satellite::{add_thread_note_tx, update_note_tx}` (`commands/note.rs`, P8.A6) | `Record`, `Confirm::Never`, so a queued agent (one that may not write the worktree) still takes notes. An agent's note lands on its own thread whatever it names; a person names one. `update_note` (a subagent filling in the note it was handed) undoes to the previous body and is refused on another thread's note. The result is `{ note, link_warnings }` — each `[[…]]` that doesn't resolve |
| `oxplow.effort.verify_claim { claim, evidence? }` / `oxplow.effort.unverify_claim { claim }` | `Tx` over `claim.evidence_ref` (`commands/review.rs`, P7.C4) | a person's review: invokers human and lens, not agent — so a lens acting for an agent is `Denied` too, by the agent policy (an agent doesn't review its own work); `Record`; undoable as each other. `verify_claim` sets the claim's evidence (`reviewer` when none is given — the person checked it) and refuses a claim already citing something; `unverify_claim` clears it. Logs `effort.claim_verified@1 { claim, effort?, evidence? }`. The claim is named by ref (`claim:7`), whatever effort it's in |
| `oxplow.effort.confirm_decision { decision }` / `oxplow.effort.dismiss_decision { decision }` / `oxplow.effort.reopen_decision { decision }` | `Tx` over `decision.provenance` (`commands/review.rs`, P7.C4) | the same reviewers. Confirm and dismiss move an `inferred` decision to `confirmed` / `dismissed` (a recorded one isn't reviewed); reopen moves either back to `inferred`, and is their inverse (and they are its). Logs `effort.decision_reviewed@1 { decision, effort?, outcome }` |
| `oxplow.config.list_keys {}` / `oxplow.config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value, `human_only` |
| `oxplow.config.set { key, value }` / `oxplow.config.unset { key }` | `Tx`: validate against the key's schema, take the new document through the loader's own validation (`oxplow_config::keys::with_key`); after commit, write the file and swap the in-memory config | undoable (inverse restores the prior value or unsets); logs `config.changed@1 { key, before, after }` (what the backend's `config.*` reactors run on); `after_commit` swaps the in-memory config, wakes `config_applied` and broadcasts the renderer's `ConfigChanged`; a **human-only key** (`HUMAN_ONLY_KEYS`: `ai`, `agents`, `agentModels`, `acpAgents`, `extensionInstances`, `activeProviders`, `replacementsOff`, `eventRetention`, `lsp`, `testing`, `extensions`, `collectors`, `agentPromptAppend` — each runs a program, picks the model, enables code, decides where work is filed, what renders a core region or how long activity is kept, or steers every agent; a test fails if a key documented as running programs or steering agents isn't listed) needs a person's confirmation per input. **Known gap (tsk997, decided to leave as is):** the gate covers `oxplow.config.set` / `oxplow.config.unset` only. A writer agent may edit `.oxplow/project.yaml` directly (the `/oxplow:configure` skill does), and the config watcher reloads the whole file, so a person-only key changed on disk takes effect without a proposal; a path guard can't close it, since Bash writes anywhere. The considered fixes — hold an on-disk person-only change no person confirmed as a proposal, or keep those keys out of `project.yaml` in local state only a person's commands write — were deferred |
| `oxplow.metric.enable { keys, enabled }` | `Tx`: turns metrics on or off — computes the new `metrics:` list with `MetricsService::apply_metric_enabled` (a bundled code metric is off until a `use:` names it; a producer/plugin metric is on until an `enabled: false` marker) and hands it to `oxplow.config.set`'s core; an unknown key (not in `metric_catalog`) is refused | undoable (restores the prior list); logs `config.changed@1 { key: metrics, … }`; the `config.metrics` reactor reseeds |
| `oxplow.metric.record { key, value, subject?, dims?, stream? }` | `Tx` over `fact_store::record_facts_tx` (`commands/metric.rs`, P4.8) | not undoable. An asserted fact on the metric's source measure, stamped to match its filter, anchored to the stream's latest snapshot; the fact and the audit commit together. A formula or `count` metric, an unknown key, or another stream (for an agent) is refused. After commit: clears the fact memo (the change loop announces `MetricSamplesChanged` for the measure) |
| `oxplow.metric.rebuild { force }` | `External` over `MetricsService::rebuild_baseline` | not undoable. Every snapshot fact collector's whole-tree baseline. It drives snapshot captures and collector scripts, which own their own transactions. (Running one collector is `oxplow.collector.sync`; `metric.run` is deleted.) |
| `oxplow.vcs.commit` / `oxplow.vcs.stage` / `oxplow.vcs.discard` / `oxplow.vcs.fetch` / `oxplow.vcs.pull` / `oxplow.vcs.push` / `oxplow.vcs.merge` / `oxplow.vcs.checkout_branch` / `oxplow.vcs.rename_branch` / `oxplow.vcs.delete_branch` / `oxplow.vcs.resolve_conflict`; `oxplow.git.rebase` / `oxplow.git.cherry_pick` / `oxplow.git.revert` / `oxplow.git.ignore` | `External` over the `Vcs` trait (`git.*`: the git provider's own ops) (`commands/vcs.rs`, P5.B6) | a person's only (`human`; agents run `git` in their terminal), not undoable. Each takes the `stream` it acts on, resolved strictly (an unknown stream is refused, never the primary's). `oxplow.vcs.discard`, `oxplow.vcs.merge`, `oxplow.vcs.delete_branch`, `oxplow.git.rebase` and `oxplow.git.revert` are `Destructive` (confirmed); `oxplow.vcs.pull` and `oxplow.vcs.push` don't ask (routine, the person's own click). The spec is the one rule: the Git Dashboard arms its confirm from it (`SpecConfirm`, tsk898). The result — and the audit row's — is the VCS's `OpOutcome { success, log, conflicts, auto_resolved }` (`oxplow.vcs.commit`: `{ success, revision }`). After a run the stream's `WorkspaceChanged` (and `VcsRefsChanged`; every stream's after a fetch, push, rename or delete) is announced. See [vcs.md](./vcs.md) |
| `oxplow.knowledge.write_page { slug, title?, body, verified_refs?, removed_refs? }` / `oxplow.knowledge.delete_page { slug }` / `oxplow.knowledge.link { page, target }` / `oxplow.knowledge.resync { slug }` | `Tx` over `knowledge::write_page_tx` / `delete_page_tx` (`crates/oxplow-app/src/knowledge.rs`, P5.C3) | all invokers, `Record` (a read-only thread captures too); not undoable; `delete_page` is Destructive. Validates the slug and every `[[link]]` (refused, named), restates the `wiki_page` row and its pinned `page_ref` edges, logs `knowledge.page.written@1` / `knowledge.page.deleted@1` with the actor's anchors; writes (or removes) `.oxplow/wiki/<slug>.md` in the run; the UI re-reads on the row's `modelsChanged` (no wiki event of its own). See [knowledge.md](./knowledge.md) |
| `<provider>.<name>` (an enabled external provider's own declared commands — not its capability's verbs, which run as `work_item.<verb>`: the fake's `estimate`) | `External` over the provider process's `invoke` (`providers/registry.rs`, P5.D3) | registered while its instance is enabled (and removed when it stops), all invokers, `Experimental`; `confirm`, `effect` and `undoable` as the provider declares (its `inverse` becomes `<provider>.<command>`). The events its `invoke` returns are logged caused by the run — only types it declares, and a `work_item.recorded` only for its own items. See [providers.md](./providers.md) |
| `oxplow.collector.sync { owner, id, thread? }` | `External` over the collector's program or script (`collector_runner.rs`, P7.B3; was `source.sync`) | all invokers, `Write`, not undoable. Runs an approved collector from the primary worktree and commits its rows, its `collector_run` row and `collector.synced@1` in one transaction (a failed run commits the failure). It never approves: an unapproved exec collector is `Invalid` at `/id`. Run by the system it's the `every:` schedule's run (`trigger: every`); by anyone else, `manual`. **The one manual run for every collector** (it replaced `source.sync` and `metric.run`): a fact collector (`facts:`, owner `project` / an extension / `built-in`) runs through the fact engine (`MetricsService::run_collector_by_key`) against the stream's latest snapshot, recording its capture, `collector_run` and `collector.synced@1` (the engine's own writes). Result `{ owner, id, rowCounts, facts }` — `rowCounts` for an entity collector, `facts` (count recorded) for a fact collector. A project **report collector** (`records:`, tsk863) is read by the collection service in `thread` (an agent's own; a person names one): `{ owner, id, recorded: { status, records, run, … } }` |
| `oxplow.provider.sync { instance, collector? }` | `External` over the provider process's `read` (`providers/sync.rs`, P7.A3) | all invokers, `Record` (it restates items, never claims), not undoable. Reads a running instance's collectors (all, or one — an undeclared one is `Invalid` at `/collector`) from their last checkpoints; the records land as `work_item.recorded@1`, each batch committed with its `$/state`. The one way to read: Settings → Integrations' Sync Now, an agent, the schedule (as the system) and the read an instance gets when it starts. Result `{ reads: [{ collector, records }] }`. See [providers.md](./providers.md) |
| `oxplow.effect.retry { effect, event }` | `External` over `effect_triggers::run_reaction` (`commands/effect.rs`, P9.D4; the `effects.run` operation, its services filled at boot when the `effect.triggers` consumer registers) | a person's only, `Confirm::Always`, not undoable. Has an extension's effect react again to an event its reaction to **failed** (its latest attempt, `v_effect_run.latest`), as the next attempt, run as the effect is now — enabled and approved as it is, composing afresh. Asked every time: a failed attempt interrupted with a step outside oxplow under way may have landed it, and a retry sends it again. `Invalid` for a reaction that didn't fail or was never made, a disabled or unapproved effect, an unknown effect or event. Result `{ effect, event, attempt, outcome, reason? }` — the retry's own outcome, which may be `failed` again. See [extensions.md](./extensions.md) "Effects" |
| `oxplow.effect.backfill { effect, from_seq? \| since?, to_seq? }` | `External` over `commands/effect.rs::backfill` (P9.D5) | a person's only, `Confirm::Always`, not undoable. Has an effect react to the matching events in the range it never reacted to (those logged before its approval), oldest first, once each, as it is now — at most 200 a run. Each is an attempt with `origin: backfill`, under the same dedupe, loop guard and health as a live reaction; three failures in a row disable the effect and stop the run. `Invalid` for an unknown, disabled or unapproved effect, or both `from_seq` and `since`. Result `{ effect, planned, ran, skipped, proposed, failed, remaining, stopped? }` |
| `oxplow.effect.backfill_plan { effect, from_seq? \| since?, to_seq? }` | `Tx`, `Read` (P9.D5) | all invokers. What `oxplow.effect.backfill` would react to: `{ effect, planned, from_seq, to_seq }`. The count a person is shown before running one (a confirmation carries a command's summary and input, not a computed count) |
| `oxplow.plugin.enable { plugin, kind, contribution }` | `External` over `plugin_health.rs` and the provider registry (P7.C1; replaced `provider.enable`) | a person's only, not undoable. Enables a disabled contribution on this machine again — clearing an automatic disable (`plugin_health` back to `ok`, its count starting over). `kind` (`provider` \| `collector`) is part of its name — a provider and a collector may share an id (tsk721); there must be a failed one of that kind, or a provider instance the registry knows. A provider instance resets its backoff and reconciles, so it starts when `extensionInstances` enables it; a collector runs at its next trigger. Logs `plugin.enabled@1 { plugin, contribution, kind }`; the result is its `plugin_health` row. See [providers.md](./providers.md) |
| `oxplow.lens.show { lens? , spec?, params? }` / `oxplow.lens.keep { answer \| spec, stream?, extension?, slug? }` / `oxplow.lens.share { lens, extension }` | `show`: `Tx`; `keep`, `share`: `External` (`commands/lens.rs`, P6.C1) | `show`: all invokers, `Record` — checks the spec's query as `query_sql` does, metric functions resolved (`SqlGateway::check`), as its pre-check before the transaction (tsk1010: so an agent can show a metric chart, called directly or composed), then its shape in the transaction; stores a `thread_answer` on the caller's thread (an agent may name only its own; a person any) with its open turn and effort; a named `lens` is the main worktree's, whatever the thread's stream, logs `lens.shown@1 { answer, thread, lens? }`; a refused query stores nothing. MCP `show_lens` runs it and returns the answer's text rendering (`text_answer`: the main worktree's lenses in the answer's own thread's lens context, the one resolution `run_answer` uses too). `keep`: all invokers, `Write`, not undoable — writes an answer, or a spec (Explore Data's Save as Lens; its shape checked as `show` checks one and its query through the SQL gateway — the metric functions rewritten, then the read contract — tsk987; in `stream`'s worktree — an agent may name only its own thread's stream, tsk988 — else the caller's thread's, else the primary's), as a private lens in `my-lenses` or another private extension — never a shared one, which is `share`'s, a person's (tsk988) — (an answer's params become defaults; `intent.origin` is the answer's thread, or the caller's), sets an answer's `kept_lens`, logs `lens.kept@2 { answer?, lens }`, and returns `{ lens, path, live }` — `live` is false when it wrote to a non-main stream's worktree, whose lenses the app shows once merged. `share`: a person's only — moves a lens into a shared extension (created with `sharing: shared` and `engine`), refused (and rolled back) when it doesn't load or its query reads anything but models. Both write lens files, which a `Tx` handler may not (the bus retries one on a busy database, and a retried file write strands the first): `keep` reads the answer in one transaction, writes the file, marks the row kept in another and removes the file when that fails; `share` must write before it can load-check. The bus records each run and its events after the handler returns. See [extensions.md](./extensions.md) → "Thread answers" |
| `oxplow.ui.report_error { label, command?, message?, exit_code?, thread?, stream?, signal?, duration_ms?, stderr?, stdout? }` | `Tx` (`commands/ui.rs`, tsk1072) | a person's only (`HUMAN_ONLY`: an agent or a lens is `Denied`, and `list_commands` doesn't offer it), `Record`, not undoable. The app's `recordOpError` runs it, fire-and-forget, for each op error it shows. Logs `ui.op_failed@1` anchored to `thread` (and its stream), or, from no thread, to the `stream` on screen so its agent can read the output (not both: `Invalid` at `/stream`, tsk1079); stderr / stdout stored as its `output` body (`put_json_tx`, namespace `ui`); the agent reads them as `v_op_error` and `read_event_content(event_id, output)` |
| `oxplow.metric.scaffold { key, title?, language?, glob? }` | `Tx`, `Read` | a starter collector script (`oxplow/collectors/<slug>.star`) and the measure + `collectors:` + metric entries; writes nothing (the agent adds the entries with `oxplow.config.set`) |

The `work_item.*` commands are **every** provider's
([work-items.md](./work-items.md)): each names its item by its canonical
ref and is dispatched to the ref's provider; an unregistered provider is
refused naming the registered ones (`no work-items provider \`issues\`;
registered: oxplow`).

**Callers.** Every task edit or status change made for someone is a
`work_item.*` command run under its actor. The desktop runs them through
RPC `run_command` as `Actor::Human` (the typed task RPCs went in P6.E1b);
an agent runs them through MCP `run_command` as `Actor::Agent` with the
caller's verified thread and stream (`McpCaller` — see "MCP identity"),
so an anonymous connection can't file or change a task, and a queued
thread can file, edit and finish tasks but not move one to `in_progress`
(tsk466). P8.A10 deleted the MCP task-write tools (`create_task`,
`file_epic_with_children`, `update_task`, `complete_task`, `upsert_task`,
`transition_tasks`, `reorder_tasks`) and `oxplow_app::task_writes` with
them; an epic is a `oxplow.work_item.create` per row (children with
`parent_ref`), and closing is the `oxplow.command.sequence` above.
`oxplow_app::work_items::WorkItems`
is a typed client over the commands (the conformance suite uses it).
**Config is written only by `config.*`** (tsk515). The Settings page's
typed IPC setters (`set_agents`, `set_agent_prompt_append`,
`set_agent_model`, `set_generated`, `set_extension_enabled`)
and `enable_metrics` each run `oxplow.config.set` / `oxplow.config.unset` as
`Actor::Human`, confirmed — the person's click is the confirmation a
person-only key asks for — through `oxplow_rpc::commands::config::set_key`.
`config_service` only reads. `only_the_config_commands_write_project_yaml`
scans the crates for any other `write_project_key` call. A key whose
new value must reach something running reacts to the `config.changed`
event on the pump, reading the value from the event's `after` (the pump
can see the event before the after-commit swap): `generated` →
`config_reactors::WorkspaceFilterConsumer` updates the snapshot captures'
filter, so an agent's change applies like the person's.
**The person's way onto the bus** (P5.A1): RPC `run_command { id,
input, confirmed }` and `undo_command { audit_id, confirmed }`
(`oxplow_rpc::commands::bus`, `Actor::Human`; desktop `runCommand` /
`undoCommand` in `api.ts`). A call that needs confirmation comes back
`NEEDS_CONFIRMATION` and the UI asks, then calls again with `confirmed`.
A typed IPC setter is a convenience over one command; anything new the
UI writes goes through `run_command`. Parity: `both("run_command")`,
`ui("undo_command")`. A person's undo is offered where they ran it: a
command run through `personCommands` (a board move, a launcher entry)
that comes back undoable (its outcome's `inverse`) shows **Undo** on its
"done" toast, which runs `undoCommand(audit_id)` — the click is the
person's confirmation — and says "undone" or records the failure
(P11, tsk975). The thrown error keeps its IPC code
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
(the specs the calling agent may run and that are offered now (step 0), with `input_schema`, `summary`,
`confirm`, `undoable`) and `run_command { id, input }` (the outcome:
`result`, `audit_id`, `event_id`, `inverse?` — or, for a run that needs
a person's confirmation, `{ kind: "proposed", proposal, message }`). Extensions never add MCP
tools. `run_command` is an agent's only write path for records — the
named write tools (`create_task`, `complete_task`, `amend_effort`,
`record_test_run`, `add_thread_note`, …) are gone (P8.A10;
`oxplow-mcp`'s `WRITE_TOOLS` lists every tool that isn't read-only,
each with its reason, and `read_write_split_covers_every_tool` fails on a
tool in neither list — a new write tool is a deliberate, reviewed
addition).

**Caller identity.** Every harness carries the acting thread on the
HTTP request, and `oxplow_mcp::McpCaller::from_parts` reads it from the
`http::request::Parts` rmcp attaches to each tool call — the
`X-Oxplow-Thread` / `X-Oxplow-Stream` headers (ACP: `McpHttp.headers`;
opencode: `{env:OXPLOW_THREAD_ID}` in its config headers; Claude: a
per-thread `mcp-config.<thread>.json` with the literal headers, since
Claude's MCP config reads no env vars), or `?thread=…&stream=…` on the
endpoint URL (Codex, whose config has no per-session headers). A
connection with neither is an anonymous agent: it may list, and
`run_command` refuses it — no run without an actor
to audit it to. **The header is a claim, not a proof**:
`OxplowMcp::verified_actor` resolves the thread and refuses an unknown
thread or a stream header that isn't the thread's stream, and the actor
carries the thread's real stream.
The wire test `crates/oxplow-control-plane/tests/mcp_wire.rs` proves the
headers reach `command.executed`'s `source = agent:thr…`.

## Adding a command

oxplow's own commands are declared in `extensions/oxplow-foundation`
over operations of the host capabilities ("Host capabilities" →
"Operations"); `Services::new` adds the operations and registers the
declarations (`register_required`). Only `oxplow.command.sequence`, the
bus's composition primitive, is registered as a Rust command. A bus with
one area's operations (a test's, `oxplow-dev`'s) registers what's
declared over them with `register_declared`.

1. Define the input as a Rust struct with `JsonSchema`
   (`deny_unknown_fields`); the operation's `input_schema` is derived
   from it.
2. Write the handler — `Tx` unless it must call a pre-existing service.
   Return the inverse (a call of the command that undoes it) when it is
   undoable, and any domain events as typed envelopes
   (`Envelope::typed::<T>`).
3. Make it an operation of the capability whose scope it falls under
   (`Op::new(capability, name, schema, undoable, handler)`, plus
   `with_confirm_for` / `with_precheck`), add the scope to
   `HOST_CAPABILITIES` if it's new (its class decides the command's
   effect), and add the op in `Services::new` (`commands.add_op`).
4. Declare the command in `extensions/oxplow-foundation/extension.yaml`:
   `name: <area>.<verb>`, `summary`, `capability`, `op`, `invokers`,
   `confirm`, `needs` (features), `ui`.
5. Route the existing RPC/MCP entry points through `commands.run(...)`
   rather than calling the service directly, so the human's and the
   agent's runs are audited alike.
6. Tests: the input schema rejection names the field; the invoker and
   confirmation rules hold; the audit row and `command.executed` share
   the transaction; undo applies the inverse.
