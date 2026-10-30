# Extensions: Permissions, run context, services and grants

Permissions and approval, the run context, services, grants and host-owned secret references. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## Permissions

An extension declares what it may reach beyond the **baseline** in its
manifest's `permissions:` (`oxplow_domain::permissions::Permissions`); a
person approves it when the extension is deployed. The baseline needs no
declaration: pages, panels, slots, decorators, replacements; its
own datasets; a collector's `input:` query (declared and reviewable); its
own commands and the effects, jobs and commands whose scripts compose them;
components that query their own datasets; Starlark and jaq collectors that
don't ask a model; `state.read`, `extension_config.read`, `jobs.read`.

```yaml
# A list, in a script's `needs:` grammar (a scope id, or `<scope>:<qualifier>`);
# absent or [] = baseline only. `oxplow_domain::permissions::PERMISSION_IDS`.
permissions:
  - sql.read                     # ad-hoc read-only SQL (a script's `needs: [sql.read]`)
  - commands:oxplow.work_item.*  # commands outside its own namespace (`*` one segment, `**` the rest)
  - files.read                   # or files.write (includes read): `needs: [tree.read]`
  - ai.compute                   # models, with the person's key
  - network:api.github.com       # hosts its programs reach (`network:*.example.com` covers the names under it)
  - process.run                  # runs programs: providers, `runtime: exec` collectors
  - secrets:GITHUB_TOKEN         # secrets it reads, by name
  - agent.write                # advisories and skills: text that reaches the agent
```

The map it used to be (`{ sql: read, process: true, … }`) is refused,
the refusal naming the list (`oxplow_domain::manifest_keys`). Inside oxplow
the declaration is still the typed `Permissions`; only its spelling is
the list — and what a deployment stores (`v_extension.permissions`,
`v_extension_deployment.permissions`, V117 rewrote the stored maps).

`process` is declared, **not confined**: a program runs with the person's
file, network and process authority (`.context/providers.md` "External
execution authority"), and what a deploy asks says so plainly.

**The loader checks the declaration against the contents**
(`extensions::check_permissions`, at the end of `load_one`): each
contribution that needs something the manifest doesn't declare is an error
at the contribution's line, naming what to add —
``provider `github` needs `process.run`, `network:api.github.com` `` in `permissions:`.
What needs what (`needed_permissions`): a provider with an `entry` or an
MCP `command` → `process`, its `network` and its credentials' names →
`network`, `secrets` (a URL adapter runs oxplow's own adapter: only its
hosts and credentials); an `exec` collector that isn't a bundled parser →
`process`, `network`, `secrets`; any declared `needs` of `ai.compute` /
`ai.discover` → `ai`, `sql.read` → `sql: read`, `tree.read` → `files:
read` (the *declared* needs: a collector's `input:` is baseline); a
Starlark collector whose script calls an `ai_*` builtin → `ai`; an
`ai_provider` implementation → `ai`; advisories, skills and
implementations with skills → `agent`; a custom component's `commands`
outside its namespace → `commands`. A script's composed commands can't be
known before it runs, so they are checked when it runs. A page's
`answers:` are suggestions a person picks, not agent text, and need nothing.

**Approval is the deploy** (`extension_deploy::approved`). The approved
set is what runs: the active deployment's permissions (the running catalog
entry's; a bundled extension with no deployment of its own runs what ships,
`shipped_permissions`). A deploy, rollback or enable whose permissions add
nothing to that goes live at once — a narrower one too. One that adds a
permission changes nothing (no copy, no row, not turned on) and returns
`DeployError::NeedsApproval { adds }`, one line per added permission
(`Permissions::added_over`: "process — runs programs with your authority;
oxplow does not confine them", "network — may reach api.github.com", …).
The command (`oxplow.extension.deploy`, `rollback`, `enable`) raises that
as `NeedsConfirmation` with the lines in `Preview::details`: a person
confirms inline (`CommandConfirm` lists them), an agent's call becomes a
proposal naming them (a deploy's consent check is a person's to approve), and the approval deploys it.
There is no separate approval store: each deployment row records the
permissions it was approved with (`v_extension_deployment.permissions`).
A fresh machine has nothing running, so a teammate's extension waits for
its first deploy here (Startup), which asks for what it declares.

**The Extension page** (foundation's `extension`, `for: oxplow/extension`,
the default): one extension as a document. Its About is a list of sentences
from `extension-facts` (where it comes from, what runs and whether the
source moved on, its permissions as the scope keys written (`sql.read`, each an
item `{text, hint}` whose hint says what it allows), a one-line Problems
summary with counts in words (contributions stopped after failing, failing,
permissions waiting for approval, load problems; none when nothing is wrong)
linking to Extension problems filtered to it) and a link to its
running `extension.yaml`; then its pages (each linking to the Definition of
the running file, `v_page.running_uri`), the other contributions by count,
what it may reach, its deployments (newest first, Roll Back to This) and the
slot `oxplow-foundation/extension.runs`, which oxplow-bundled fills with
`extension-runs` (foundation never depends on bundled). Deploy, Roll Back,
Enable, Disable, Share with the Team, Stop Sharing and Remove are the page's
actions (the ☰ menu), each offered by a `can_*` column of its first loader;
one that adds permissions asks inline like any deploy.

**The Extensions page** (`extensions`, the launcher's System page; Settings
has no Extensions area) is a document too: one headed section per extension
(`viz: sections`, a row per extension from `extension-facts`' `summary`: an arrow after the heading opens its Extension page, the facts are columns, a Problems line is the body; an extension turned off has " (disabled)" after its name),
those needing attention first, each fact a sentence from the hidden
`extension-facts` page — the one place that turns `v_extension`,
`v_extension_source` and `v_contribution_health` into words (a scope keeps its key; its explanation is the hover). It has no table and no buttons.

**Extension problems** (`extension-problems`, System) collects everything
wrong with an extension: a contribution stopped after failing (never called
"disabled": that word is for an extension turned off) or failing, permissions
waiting for approval (`v_extension.pending`; the person approves with Deploy on
its page), and load errors. The extension is the `sections` group (its arrow opens
the Extension page) and each problem a subsection under it (titled `<kind>
<contribution>`, `Did not load cleanly`, or `Needs your approval to run` with the
waiting permissions as `format: items`), with Start Again,
Repair with the Agent and Start All Stopped. A link from a one-line summary narrows it with the view param
`#extension=<name>`; Alerts' failing-extension rows link the same way.

What a contribution is, in words, is read from its kind's own dataset, which
joins `v_contribution` on `uri`: `v_command` (summary, scope, confirmation),
`v_skill` (the frontmatter description), `v_slot` (a page mount: host, slot,
page) and `v_custom_component`, all live; the Extension page's contributions
use them as section bodies.

`v_extension.permissions` is the declaration as JSON (`{}`: baseline);
`v_extension.pending` is what an undeployed extension waits on a person
for (the same lines; `[]` when nothing).
The shipped extensions declare theirs like any other: `oxplow-foundation`
`{ ai: true }` (its AI providers), `oxplow-bundled` `{ sql: read, ai: true,
agent: true }`.

## The run context

Every run of extension code — and every bus run — carries one
`RunContext` (`oxplow_domain::run`; the app's constructors are
`run_context.rs`): its run id (a UUIDv7), its parent run, the
contribution it belongs to (its URI, the extension's
`version`, the catalog generation it was built against), its actor,
anchors, subject and cause, its mode, its budget, its cancel flag and
its scope trace.

- **Built by a named constructor, never assembled.** `for_effect`,
  `for_policy`, `for_dry_run` and
  `RunContext::core` fill what that run has and leave the rest `None`.
  Nested work is `child()`: a new run id with this one as parent, the
  same actor, cancel flag and generation, the budget no larger than
  what's left. A child can be another contribution's (a collector a
  person's `oxplow.collector.sync` ran) but never another actor's —
  `placed_in(stream)` only fills an agent's stream its transport left
  out.
- **Modes are explicit.** `Pure` (a composer, an effect's or policy's
  script: it may be run again, no I/O but scope reads), `Transactional`
  (a `Tx` handler), `External` (may call a program, a provider, a
  dataset). What a mode may use is the services table's (`modes`), enforced
  by `RunContext::decide_service` ("Services" below): a pure run asking a
  dataset is refused before its script runs.
- **A run is checked for authority, not for staleness.** `check_authority()`
  refuses a cancelled run or one a live guard fails (and cancels it). A
  generation going out of date is the activation's read lock (a run that pins
  a generation holds it; one that doesn't belongs to its extension's
  deployment, `run_context::live_owner`), so a run carries no "stale" outcome: it ends
  `cancelled`.
- **Who carries it:** the bus (`run_in`, `TxCtx::run`, `Invocation::run`
  — an `External` call's run is a child of the command's), scope calls
  (`scope_calls::Calls::new(run, needs, read)`, counted on its trace),
  a composer (`Composer` gets a pure child; what it calls becomes the
  run's scopes), effects (`for_effect`: the effect actor on the event's
  thread and stream, caused by the event), the effort policy
  (`for_policy`, its composed calls `run_in` it so `command.executed`
  is caused by the event), entity and fact collectors (a child of the
  run that ran them, named for the collector), provider invokes (on the
  wire as `InvokeParams.context`; `host/call`s are answered only inside
  a registered run) and knowledge, snapshot and harness verbs. A page
  acting is `Actor::Page { page_id, on_behalf_of }` (source: the page's
  URI; a node's id is in `actor_id`) and a component acting is
  `Actor::Component` (source: its contribution ref); both act for the
  viewer.
- **Composed commands keep child ownership.** Routing captures each child's
  registered contribution factory with its retained handler. Nested composers,
  transactional leaves and external steps receive that child's context, not
  the parent extension's namespace or declaration. Transactional leaves narrow
  their mode; all children inherit cancellation and live guards. Native scope
  operations capture their implicit backing scope in declared authority.
  Routing and dispatch check run authority. This is handler attribution, not
  a coordinated DB/catalog activation or a new per-child command audit.
- **Every contribution's run is recorded once** (`extension_run`,
  `v_extension_run`; [DATA_DATASET_X.md](./DATA_DATASET_X.md) "extension_run"):
  its identity, actor, anchors, cause, outcome, cost (`cpu_ms`,
  `peak_bytes`, from each script's `Ran`), the scopes it called and its
  model calls — never what it read or wrote. Nested work of the same
  contribution adds to its parent's usage (`child()` shares it); another
  contribution's child is its own row, its `parent_run_id` the run that
  started it. A person's `oxplow.effect.retry` / `backfill` links the
  reaction's run to theirs (`under`) without making it theirs: the
  effect still runs as itself.
  **Reading it:** the bundled `runs` page (`oxplow-bundled/runs`, titled
  "Extension runs", in the launcher under Activity) lists, per extension
  and for the last 7 days, its runs, failures, denials, mean `cpu_ms` and
  model calls, the ones needing attention first. The Extensions page
  shows each extension's worst current run (`v_extension_run`, `latest =
  1`). Opening an extension's row (the hidden `oxplow-bundled/extension-runs`
  page, param `extension`) lists its runs one by one for the same week:
  contribution, version, generation (blank for a run that holds none),
  actor, causing event, outcome, error, wall and CPU time, model calls — the
  small inspector, with no log or secret to read.
- **How agents use oxplow's interface.** The bundled `agent-interface` page
  (launcher, Activity) is a dashboard of three tables over the last 30 days:
  `interface-tool-calls` (oxplow's MCP tools by tool and harness, from
  `v_tool_call` joined to `v_agent_session`; a call whose session oxplow has
  no record of reads harness `unknown`), `interface-commands` (agents' runs by
  command from `v_command_audit`: runs, invalid, denied, failed and the share
  not ok; opening a command shows the hidden `interface-command-errors` page,
  its most common error texts counted together up to the "Expected one of"
  list) and `interface-hints` (`v_hint_stat` per hint, those that never fired
  first). The bundled `hint-never-fires` advisory tells the person, once per
  hint, about a hint evaluated 100 or more times that never fires.
- **Failure, retry and freshness are the same everywhere it matters.** A run
  that ends `failed` counts toward the contribution's failure policy (three in
  a row disable it; `v_contribution_health`); one that ends `cancelled` — a
  person's cancel, or its extension changing or being disabled mid-run — does
  not (an effect's script is checked once more when it returns, so a disable
  during it is a `cancelled` run and a `failed` attempt that doesn't count,
  `Reacted::Stopped`). What a *refused* run is: `denied` (a caller or grant the contract
  rejects) is never retried; a *failed* effect attempt is sent again by itself
  only when it was `Unavailable`, every step it composed goes to a provider
  keeping `idempotent_writes`, and it has retries left (otherwise a person
  retries it); a job replays only if it declares `replay: idempotent` (up to
  three attempts); a collector is not retried by itself, its schedule runs it
  again. A collector whose last run failed is not `fresh` in
  `v_contribution_health` — its rows are the last good run's — and a failed
  attempt keeps its own `extension_run` row, reachable from its data effects
  (`effect_run.run_id`, `collector_run.run_id`, `job_run.run_id`). A program's
  words in an `error` have the credentials it was handed replaced. Runs are
  kept 90 days (`EXTENSION_RUN_DAYS`). Pinned by `extension_run_health_tests.rs`
  and `event_retention::tests::extension_runs_past_their_window_are_swept`.
- **Cancel reaches the worker.** Every script a collector, command,
  effect, policy or fact collector runs is passed its run's cancel flag;
  an exec collector's program is killed when it's set. Upcasts, report
  parsers and the MCP adapter's mappings still run
  under their own fixed budgets without one.

## Services

A script reaches oxplow through **services**, each a scope it names in
its `needs` (`oxplow_domain::service::SERVICES`, one table every reader
uses):

| Service | Builtins | Needs permission | Modes |
|---|---|---|---|
| `sql.read` | `scope("sql.read", …)` | no | any |
| `tree.read` | `files`, `source_files`, `duplicate_blocks` | no | any |
| `ai.compute` | `ai_classify`, `ai_score`, `ai_summarize`, `ai_extract`, `ai_complete` | `ai` | external only |
| `ai.discover` | `ai_capabilities`, `ai_models`; `scope("ai.discover", { role })` | no | any, when the host supplies AI metadata |

- **Who may declare what** (checked at load): a Starlark collector's
  `needs:` any service (`check_declared`; another runtime has no
  builtins, so `needs` there is an error); a composing script — a
  command's `entry:`, an effect, an effort policy script — reads only
  (`scope_calls::check_script_need`: it runs pure, so `ai.compute` is
  refused, naming why). A **command's** script reads the semantic layer
  (metric functions included) and not the tree: it decides what runs,
  and reading a whole tree belongs to an effect or a collector (refused
  at load, naming them). A provider's
  `needs` stay `scope()` calls (`check_callable`: `host/call` answers
  `sql.read`).
- **`scope("ai.discover", { role })`** (callable from an effect, a command
  or any composing script that declares it) says whether a dataset serves
  `role` — a built-in, or the bare id of one of the run's extension's
  `ai_profiles`, qualified as `Role::parse_in` does — without spending:
  `{ role: "<qualified>", model: { provider, model }, problem: null }` when
  bound (an unbound profile reports its fallback's binding), or `{ role,
  dataset: null, problem: "<why>" }` when no dataset is bound or no active
  extension declares the profile; any other settings failure is an error.
  It reads settings only (`AiService::binding_for`): no key, no capability
  probe. An effect uses it to skip, with a readable reason, instead of
  failing for want of a model. `answers: { ai.discover: [...] }` stand in
  for fixtures.
- **What a kind implies:** a Starlark fact collector reads its tree
  (`CollectorSpec::service_needs` adds `tree.read`); an entity collector
  whose script names a model-calling builtin uses `ai.compute`. Discovery
  builtins require an explicit `ai.discover` declaration. A collector's
  descriptor lists declared ∪ implied.
- **What a script uses** is read from its syntax
  (`oxplow_script::uses::services_used`, which `calls_ai` is a view of),
  and **checked at load**: `services::check_uses` refuses a script that
  uses a service its needs don't name (a collector's needs include what
  its kind implies), or one its kind may not use
  (`scope_calls::check_script_need` / `check_command_need`) — a load
  error at the script, "uses `files()` without `tree.read` in its
  `needs` — fix: …". The router's refusals are the run-time backstop.
- **Discovery.** `services::of_extension` lists each script's services
  (declared / implied / undeclared, used or not, permission) —
  `CheckReport.services`, `oxplow extension check --services`; the live dataset `v_scope`
  lists every scope (id, access, host, summary) for an agent; `docs/reference/extensions.md`
  "Services" is generated from it (`the_reference_lists_every_service`,
  sharing `test_fixtures::generated_section` with the kinds table).
- **Where a person sees them.** the Extensions page shows a line per
  script under each project extension's row — "Collector triage — Read the
  captured tree · Ask a dataset (you approve it)" — worded from the service
  titles (`list_services`, one rpc over `SERVICES`; the surface-parity
  manifest has it as a UI read, with `v_scope` for the agent), a service its script uses without naming it
  in `needs` flagged "(not in its needs)". The row's lines come from the
  extension's check (`oxplow_validate_extension` → `CheckReport.services`).
  Bundled extensions show no
  line (they are not checked in the section). Row dataset:
  `serviceLines` in `extensionRowModel.ts`. `oxplow extension new
  collector` scaffolds `needs: []` under a comment naming the three
  services.
- **At run time, one router** (`crates/oxplow-app/src/services.rs`):
  every host that runs a Starlark script — entity collectors
  (`collector_runner::given`), fact collectors (`MetricsService::given`),
  commands (`compose_calls`), effects (`effect_triggers::run_script`,
  `effects::run_over`) and effort policy scripts — calls
  `services::decide` / `prepare` with the run, the script and its needs.
  For each service the script uses it refuses what the run's mode
  excludes (`RunContext::decide_service`: "a pure run can't use
  `ai.compute`"), then what `needs` doesn't name ("`files()` needs
  `tree.read` in this script's `needs`" — a run with no authority too),
  then what the authority denies — recorded on the run either way — and
  builds the rest from the caller's `Sources`: the tree (`Trees::From`
  a `TreeSource` — the fact engine's, which reads the run's anchored
  snapshot, else its stream's latest, else the primary stream's, and
  keeps the last one — or `Trees::Fixed`, a fact collector's sweep or a
  fixture) and the dataset (a `CollectorOracle` on the run, caller
  `collector:<owner>/<id>`). A `sql.read` call is decided as it's made
  (`Calls::serve`), not twice. A caller with nothing to give (a command;
  a dry run, which is pure) says the service isn't available there. The
  worker gets the result as its host set (`Given::onto`).
- **Permission** stays the host's: a script that names a model-calling
  builtin needs the extension's `ai.compute` (the loader refuses it
  otherwise, "Permissions"), for an entity collector and a fact collector
  alike.
  Metadata-only discovery does not spend a key or require that permission;
  it still checks the declaration and live run authority. It never accesses
  credentials or probes a model. See [ai-providers.md](./ai-providers.md).
  Contained entity and fact collectors also pin their loaded script text to
  the current entry; text loaded for an older deployment cannot answer a
  subsequent service call or final result.

## Grants

A **need** is what a contribution declares; a **grant** is what its
extension may reach (its `permissions:`, approved when it was deployed —
"Permissions"); a **decision** is what a run was allowed
(`oxplow_domain::grant`, `grants.rs`).

- **The request.** `GrantRequest { scopes, program, hosts, credentials,
  env, commands }`, derived from the declaration: an exec collector's
  entry, `network`, `credentials` and `env`; a collector's services
  (`services::collector_needs`: declared ∪ implied, `ai.compute` among
  them for a model-calling script); a script's (effect, command, effort
  policy, job, provider `host/call`) scopes from its `needs`.
- **The grant.** `Grant { namespace, permissions }` (`grants::grant_of`):
  the extension's command namespace and its declared permissions. There is
  no approvals store and nothing hashed per program: the deployment is
  immutable, so what runs is what was deployed, and the deployment's
  permissions are what a person approved.
- **The authority** a run carries (`RunContext::authorized`,
  `run::Authority { declared, grant, enforcement }`,
  `grants::authority(ext, declared)`). Every extension run has one: a
  collector, an effect, a job, a script command, a scripted effort policy,
  a provider's `host/call`s. Core's own work has none and is allowed what
  it does (core's built-in effort policy, core commands).
- **Decisions.** `run.decide(Need)` → `Allowed`, `Denied(why)` or
  `Unenforced(why)`, recorded on the run (`v_extension_run.decisions`).
  `grant::decide` refuses what the contribution didn't declare ("not
  declared"), then what its extension's permissions don't allow — the
  refusal names the permission: ``needs `ai.compute` in the
  extension's manifest`` (`Grant::permits`: `sql.read` → `sql`;
  `tree.read` and paths → `files`; `ai.compute`/`ai.discover` → `ai`; a
  program, its files and env → `process`; a host → `network`; a
  credential → `secrets`; a command outside the extension's namespace →
  `commands`). Then `Enforcement { hosts }` says what this machine can't
  hold it to: a program's files always, its hosts wherever the network
  isn't confined — `Unenforced`, said rather than passed off as held.
  Decided: each scope call (`Calls::serve`); a program collector's
  `program`, `files`, each `env` name, each `host` and each `credential` —
  one not permitted is never read from the keychain; a script's services
  (`scope` `tree.read` / `ai.compute`, by the router). A denied need stops
  what asked for it. A model-calling Starlark collector or fact collector
  is refused at its `ai.compute` decision when its extension lacks `ai`
  (`RunCollectorError::Denied`, recorded as the run's `error`).
- **What the loader already refused.** Most of this is decided before
  anything runs: an extension whose contents need a permission it doesn't
  declare doesn't load ("Permissions"). The runtime decisions are for what
  only a run reveals — a script's composed commands, a path or dataset it
  reads.
- **Bounded reads.** `sql.read:v_work_item` names an exact published
  dataset; use multiple needs for multiple datasets. SQLite preparation checks
  the query's datasets and transitive registered `ref` inputs, including
  materialized datasets, before evaluating rows. Bare `sql.read` preserves
  the explicit whole-layer grant. Bounded queries cannot currently use
  `metric_grid` / `metric_findings` because their host precomputation has
  no dataset authority contract; query granted published datasets instead.
  `tree.read:src/main.rs` names a captured file; `tree.read:src/` names a
  directory. Absolute paths, traversal, backslashes and globs are refused.
  The host filters its captured tree before giving it to a script; these
  needs do not grant raw filesystem access. Bare `tree.read` grants the
  whole captured tree. A collector's `input:` query is the host's, under
  its declared dataset bounds, and needs no `sql` permission.
- **External execution.** Every external program (an exec collector, a
  fact program, a report parser, an external provider, a harness or MCP
  stdio server) runs from its extension's deployment — working directory
  and `OXPLOW_EXTENSION_DIR` both — under the `process` permission, with
  the person's file, network and process authority, and a deploy that adds
  `process` says so plainly. Confining external processes is shelved (the
  owner's decision); there is no `execution:` key. What oxplow does do is
  cleanup, not confinement: on Unix each program runs in an owned process
  group, killed on completion, timeout, cancellation or when its extension
  stops being live; a process can escape its group. Pipe I/O has a
  deadline, so escaped children cannot leave host threads blocked.
  Credentials are handed to a process at spawn and are process-readable;
  contained script services never receive keychain values. Disk extension
  reads refuse special files and files larger than 16 MiB; Unix opens
  refuse a final symlink and avoid blocking on replacement FIFOs.
- **Live checks.** A run that outlives a generation holds a
  `run_context::LiveCheck` (`live_owner`: its extension still enabled, the
  same deployment, still declaring it), composed through nested runs and
  rechecked before sensitive calls and publication. A program's process
  group is killed when it fails (`external_process`); an in-process script
  runs under `run_context::watch_authority(run)`, which sets the run's
  cancel flag (the script worker polls it). Disabling or redeploying an
  extension therefore stops its running work. This cannot undo an external
  side effect already performed.
- **Collector previews.** A preview stores no rows, but receives the same
  authority as an ordinary run; it never runs a program.
- **Component authority.** Query messages carry the loaded bundle version.
  Asset and command allowlists come from that bundle; a component's
  commands outside its namespace need `commands:<pattern>` (the loader
  checks them).
- **Not yet:** OS confinement and model call-count/monetary budgets.
- **Deliberately still open:**
  - adversarial tests for bounded `sql.read` (CTEs, UNIONs, table-valued
    and pragma functions, the `metric_grid()` refusal) and bounded
    `tree.read:` (sibling prefixes, `..`);
  - a shebang's interpreter resolves from `PATH`;
  - credentials are handed to a process at spawn: stopping it can't take
    a credential back, and a keychain deletion mid-run isn't watched;
  - a run built without authority is unbounded (core's own runs, by
    design; not yet a tested invariant);
  - network confinement is `sandbox-exec` on macOS only.

## Host-owned secret references

`oxplow-domain/src/extension_secret.rs` defines a strict version-1 identity:
`{schemaVersion:1,id:<canonical UUIDv7>}`. It contains no owner, path, keychain
account, secret bytes or authority. References may appear in state/config;
values remain in the machine keychain under the primary project and current
owning extension. Copying team configuration cannot approve use or read another
namespace. Disabled/uninstalled owners cannot save or inspect a reference;
explicit human revocation can delete a retained keychain value after uninstall.

`ExtensionSecrets` keeps the values; whoever sets one does it with a command,
`oxplow.extension.set_secret { extension, reference?, value }` (a fresh
reference when none is given) and `oxplow.extension.revoke_secret { extension,
reference }`, scope `credentials.write`. A secret is a kind of value, not a
kind of actor (commands.md "Secrets"): `value` is `writeOnly`, so a person, an
agent and a page for either may set one, and no record, event, result or
dataset carries it. Presence is the live dataset `v_extension_secret` (`extension`,
`provider`, `name`, `reference` as JSON, `is_set`), read from the catalog's
provider credentials and the keychain; it follows `CredentialChanged`. Secret
bytes and keychain error details are never returned or audited. Values are
bounded to 16 KiB. Settings → Integrations lists them in the foundation's
`extensions-secrets` node (Set… / Rotate…, a form over the command whose
`value` field is masked by the schema, and Revoke). The old
`save_extension_secret`, `extension_secret_is_set` and
`revoke_extension_secret` IPCs are gone.

The first trusted consumer is a remote MCP HTTP provider using the shipped
adapter. Its bearer credential must be named `OXPLOW_MCP_BEARER` and carry
`secretRef`; references are refused for package executables, stdio servers,
OAuth credentials and other environment variable names. The fixed name avoids
process environment control variables. The reference identity, endpoint and
declaration are part of the deployment. The host checks exact current
owner and permissions before and after keychain resolution and gives bytes only to the
shipped HTTP transport; mapping scripts get no secret reader. Redirects are
disabled in that transport. References remain project-local even for globally
configured provider instances.

Processes capture a host reference revision at admission. Save/revoke invalidates
that revision synchronously, refusing further sensitive callbacks and causing
the existing process revocation watcher to stop old execution. Consumer cleanup
is detached from the settings RPC's lifetime, then requests coordinated
reconciliation. Keychain mutation failure also invalidates old consumers; it
does not claim deletion succeeded. Runtime isolation/revocation checks remain
pending.
