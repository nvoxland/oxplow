# Extensions: Commands, implementations, skills, advisories and agents

Extension commands, capability implementations, skills, advisories and the agent MCP surface. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: One skill set

- **One skill set.** Every agent runtime writes its skills and commands
  from one `AgentText` (`oxplow_domain::agent::text`): core's
  (`OXPLOW_SKILLS` and `CORE_COMMANDS` in
  `crates/oxplow-agent-text/src/lib.rs`) plus what
  extensions offer now ("Skills").

## Commands

An extension's `commands:` (a stable kind, P6b; `extension_commands.rs`)
are commands on the bus whose handler is a Starlark script that
**composes core commands**, reaching anything else only through the **scopes**
it declares in `needs` ([commands.md](./commands.md) "Scopes") — `scope("sql.read", { sql, params })` today.
A script's scope needs are the scopes a script can call
(`scope_calls::CALLABLE`, `check_callable`): one it can't is refused at
load, the one rule a command's `entry:`, an effect and a provider's
`host/call` meet. A capability need (`work_items.comments`) gates the
command instead.

`sql.read` returns complete row lists only: more than 1000 result rows
fails in `scope_calls::Calls` before composition can act on a partial
set. Exactly 1000 is allowed. A script that means to walk a large result
asks for a `page: { limit, offset }` and gets `{ rows, more,
next_offset? }` ([commands.md](./commands.md) "Scopes"). Aggregate in SQL when all matching data
must contribute, or explicitly filter/limit a query when a subset is
intended. Commands, effects, effort policies, provider host calls and
dry runs share this check. A complete query result does not establish
collection coverage or freshness; the row-only scope carries neither.

```yaml
commands:
  - name: review.finish                # <area>.<verb>, each [a-z][a-z0-9_]*; id <namespace>.review.finish
    summary: Mark the task done and leave a note.
    input_schema: { type: object, required: [uri], properties: { uri: { type: string } } }
    entry: handlers/finish_review.star # defines transform(x), x = { input }
    confirm: never                     # never (default) | always | destructive; children only add
    access: write                      # write (default) | record | read (composes nothing, returns a result)
    job: false                         # true: a durable job's model may call it (needs entry:, never over a scope's operation)
    needs: [sql.read, work_items.comments]   # scopes it calls; capabilities / features it needs active
    ui:                                # how a person meets it (`.context/commands.md` "Offering a command")
      label: Finish Review
      group: Review
      about: oxplow/work_item          # resource-bound; absent: search offers it
      # about_when: closed             # only where this column of the resource's dataset row is truthy
      input: { uri: "{{uri}}" }
    examples:
      - { name: happy, input: { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }, expect_commands: [oxplow.work_item.transition] }
      - { name: gone, input: { uri: "oxplow:oxplow/work_item?id=tsk9&provider=oxplow" }, answers: { sql.read: [[]] }, refused: no such }
    example: { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }   # the input an agent is shown; a script's defaults to its first example that runs
```

`example:` is the command's example input, for any handler (script,
scope operation, provider op): it registers as JSON Schema's `examples`
on the input schema the command ends up with, an operation's schema
included, so `list_commands { id }` shows it to an agent and a refused
input quotes it ([commands.md](./commands.md) "Exposure to agents"). A
script command without one uses its first example that runs (not one
expecting a refusal). Every command an agent is offered has one that
fits (`every_command_an_agent_is_offered_has_an_example_that_fits`).

A command may be **backed by an operation of one of the extension's
providers**: `provider: fake` + `op: estimate`, the operation one its
checked-in declarations list (not a capability verb — those run as
`oxplow.work_item.<verb>`). Its input schema (plus an optional
`instance`), access and undo are the declaration's, its `confirm` the
stronger of the manifest's and the declaration's; it runs on the
instance its input names ([providers.md](./providers.md) "The host").

A command may instead be **backed by an operation of a scope**
([commands.md](./commands.md) "Scopes" → "Operations"):
`scope: bookmarks.write` + `op: set` in place of `entry:` — no
`input_schema`, `access` or `examples` (the operation's and its
scope's access), the scope implied in `needs`; the bus refuses
to register one naming an operation that isn't there. oxplow's own
commands are declared this way in the required `oxplow-foundation`.

A bundled extension's commands are `Stable`, any other's
`Experimental`. `transform` returns `{ commands: [{ name, input }], result?, events? }`, or
`{ refuse: "<why>" }` to decline — the run is `Invalid` with that reason
and writes nothing (`composed` → `Composed::{Run, Refused}`). An `access:
read` command composes nothing and logs no events (refused at run time
otherwise): it reads and returns a `result`, is never confirmed and needs
no scope that changes things; the bus runs it unrecorded. An example may give `answers:` — per scope, its calls'
answers in order, standing in so it doesn't depend on the project's data
(a scope without any is served for real; one that runs out is an
error) — and may expect a refusal with `refused: <part of the reason>` instead of
`expect_commands` (not both). A command's manifest `name` is
`<area>.<verb>`; its **command id** is `<namespace>.<area>.<verb>`. The
**namespace** is the manifest's `namespace:`, else the extension's name with
`-` → `_` (`command_namespace`; `check_namespace`: one snake_case segment,
and `oxplow` only for an extension that ships with oxplow — the bundled one
declares it, so its commands are `oxplow.review.accept`,
`oxplow.review.request_changes`, `oxplow.work.clear_finished`). Its event
types are declared by full name and unaffected. Who holds a namespace is the
bus's to say (`CommandBus::namespace_owner`: `oxplow`, or the one source —
`extension:<name>` / `provider:<instance>` — holding it) — there is no
hand-kept list: checked against the running registry (`RunningCommands`,
which the bus implements), a namespace another source holds is an error, and
at run time the generation's namespace reservation refuses it all-or-nothing
under one lock (reported as the extension's problem). The registrations key
registrations by extension (several of oxplow's share `oxplow`) and removes a
disabled one's with `unregister_source`. Two enabled extensions declaring one
namespace (other than `oxplow`) are both refused at load
(`refuse_shared_namespaces`, after disabling applies). Each entry is
checked at load, its error at its line (`entry_line`: the line whose
`name:` is exactly that name — `a` never lands on `abc`; ids, pages,
and datasets find their lines the same way): the name, `access`, `confirm`,
the schema compiles, each `needs` entry is a scope or a
capability / feature, and the entry is a file in the
extension that parses and defines `transform` (`check_starlark`), and
it declares at most `MAX_EXAMPLES` (10) examples. The
script's text is kept on the `ExtensionCommand` (not serialized).
`check_extension` (the Extensions page, `oxplow extension check`) runs
`check_commands`: with a running oxplow's registry, each example is
dry-run (`dry_run`) — `compose_calls`, its `sql.read` calls answered by the
example's `answers` or through the check's `SqlGateway` (its overlay
included, so a read of a raw table is an error at the example), and what
it composes against the registry — every command exists, its input
fits, the names are `expect_commands` in order. Without a registry it
warns that the examples weren't checked. **`compose_calls`** is the one
compose step, for the dry run and the handler alike: the script in the
sandbox under `COMMAND_SCRIPT_BUDGET` (5 s of the script's own time, not
the collectors' 120 s runaway catch: the bus composes it once, on a read
snapshot it holds while the script runs; its services are the router's,
`services::decide` — `sql.read` only, see extensions-permissions.md "Services"), its `scope`
calls served on the calling thread (`run_command_script` with
`ScriptRun::with_scope`, through `scope_calls::Calls`; a busy database
retries the run), then the
`{ commands, result? }` shape (`composed`; any other key is `Invalid`).

**Running** (`extension_command`): each is a composite
(`Handler::Compose`, atomicity `Dispatch`) `<namespace>.<name>`
(summary "… (extension `x`)", the declared confirm / access,
`Lifecycle::Experimental`). Its composer runs `compose_calls` on the bus's routing thread,
answering `sql.read` through the SQL gateway as an effect's script is
answered (`SqlGateway::run_in`: the `query_sql` authorizer, row cap and
timeout, the metric functions computed first), `state.read`,
`configuration.read` and `jobs.read` through the gateway's sources, and
counting each call in the run's `ScopeTrace`. It runs once per run, when
the bus routes it.
The bus runs what it composes ([commands.md](./commands.md) →
"Composition"): when every call stays in oxplow's records, as children
in one transaction (`steps::run_composed`; the script runs once, when the bus routes the run) — each child's policy and
confirmation apply (a person's run whose child asks is asked first; an agent's run asks nothing), one audit row with `{ result,
children }`, the children's events caused by the run, the reversed
children as its undo; when one leaves it (`work_item.*` on another
provider's item), as **steps** — all checked first, then run in
order, each landing as it runs, stopping at the first failure, one audit
row, no undo. A script can't do I/O of its own: `files()` sees
nothing, `ai_*` is refused without a host, and `scope` reaches only
what it `needs`. **Registration**
(`ExtensionCommands`, `Services.extension_commands`): the enabled
live extensions (one bus, like providers — a
command registers when its extension is deployed), one
namespace at a time, all-or-nothing; registered by the generation at boot and on the
extension catalog's change signal (`ExtensionCatalog::changes`, P7.B6):
a deploy, rollback, enable or disable publishes it, and the
`config.extensions` reactor fires it for the `extensions` key.
Editing a source fires nothing. The extension datasets, the provider registry and
the metric catalog follow the same signal; a namespace something else
already holds (a provider) is refused and kept as the extension's
`problem`. Replacement is prepared against a cloned command registry and
published with one write-lock swap; rejection keeps the previous command batch.
Missing/disabled sources are removed. Registration tracks namespace, declarations,
package version and catalog identity, so provenance changes rebuild the batch
without requiring a script change. Each handler captures version and generation
from the same `CatalogSnapshot` as its declarations; a retained handler never
looks up a newer candidate's identity when invoked. Other registries and SQL
datasets still refresh independently. A launcher `{ command }` entry may name one.

## Implementations

An extension declares capability implementations (stable,
`implementations:`; `extensions/implementations.rs`):

```yaml
implementations:
  - { capability: work_items, id: oxplow, entry: "oxplow:tasks" }
```

- `capability` is a choosable one, or one many implementations serve
  (`oxplow_domain::capability`: `agent_harness`,
  `ai_provider` — every one declared is active); `id` is what
  `activeProviders` names, or a many-capability's key (an agent session's
  `harness`) — lowercase, never `none`.
- **`icon:`** (any capability) is a path to an SVG inside the extension
  (`icons/claude.svg`): the implementation's logo, shown wherever it is
  (an agent harness's, on a session's tab). The checker (`load_icon`)
  refuses a missing file, a path outside the extension or not ending
  `.svg`, a file over 16 KB, one that doesn't start `<svg`, and one
  carrying `<script`. The declaration keeps it as a `data:image/svg+xml;base64,…`
  URI (`ImplementationDecl.icon`, `Implementation.icon`); the UI draws it
  only as an image (the mask of an empty element, `AgentIcon`), never as
  markup, so nothing in it runs. The foundation extension ships a
  monochrome mark each for `claude`, `codex` and `opencode`
  (`extensions/oxplow-foundation/icons/`); an external `agent_harness`
  provider declares its own the same way.
- `config:` configures the built-in, checked at load against its schema
  (`BuiltIn.config_schema`, `capability::check_config`; a built-in with
  none refuses any config); no built-in takes one now.
- `entry` names a built-in in core's standard library
  (`capabilities::BUILT_INS`: `oxplow:tasks`, `oxplow:commit-or-switch`,
  `oxplow:snapshots`, `oxplow:snapshot-hashes`, `oxplow:wiki`), the way a collector names
  `oxplow:junit`. A
  built-in's features are core's table's — it's core's code — never the
  manifest's. A built-in whose items' URIs carry a provider id
  (`BuiltIn.provider`: `oxplow:tasks` is `oxplow`) is declared under that
  id and no other.
- **A process implements a capability through `providers:`**, not
  here: a capability whose spec carries a provider contract (the work
  list, the effort policy, an agent harness) may be an external
  provider's instance — the work list and the effort policy chosen by
  `activeProviders` like a built-in, a harness registered beside the
  built-ins under its instance id, as many as run
  ([providers.md](./providers.md) "What a provider may implement").
- **An `ai_provider` names its Rust kind** with `provider:` instead of
  `entry:` (`implementations::provider_decl`, `Source::ProviderKind`):
  one of `oxplow_ai_rig::KINDS` (`anthropic`, `openai`,
  `openai_compatible`, `openrouter`, `gemini`, `mistral`, `groq`,
  `deepseek`, `xai`, `ollama`, `llamacpp`, `typesafe`), an unknown one
  refused naming them. It takes `config: { baseUrl? }` (any other key is
  refused), no `needs`, and needs `ai.compute` (extensions-permissions.md "Permissions"): it
  points a person's key at that URL. Its `decide_native` feature is the
  kind's. The kinds and why they're Rust over Rig are
  [ai-providers.md](./ai-providers.md) "Rust kinds over Rig".
- **An `effort_policy` may be a script** instead: an `entry` that isn't
  `oxplow:<name>` names a `.star` file of the extension
  (`implementations::script_decl`, `Source::Script`). Nothing else is a
  script.
  - An effort policy's is effects-shaped — `transform({event})` →
    `{ commands: [{ name, input }] } | { skip: "why" }` — takes no config,
    and names the scopes it calls in the entry's `needs:` (only what a
    script can, `scope_calls::CALLABLE`; checked at load, with the script
    defining `transform`). `needs` is refused anywhere else. It runs
    under its extension's permissions, as an effect's does; what runs is
    what was deployed.
- `oxplow-foundation` declares the agent harnesses (`oxplow:claude-code`,
  `oxplow:codex-cli`, `oxplow:opencode`) and the AI
  providers, one per Rust kind under the kind's name, each at its API's
  default URL; foundation is required, so
  `CapabilityRegistry::new` registers nothing for them. The harnesses are
  registered by declaration (`harnesses::prepare_built_ins`, published by the
  generation → `Services.harnesses`, an `oxplow_domain::agent::registry::HarnessRegistry`).
- A required capability's default is core's own: `CapabilityRegistry::new`
  registers it (snapshots' `oxplow`, `oxplow:snapshots`), so it's there
  whatever is disabled, and a manifest declaring that id is an error.
- `oxplow-foundation` declares the default implementations (the work list,
  the effort policy and knowledge's wiki, `oxplow:wiki`), so they run with
  `oxplow-bundled` off; turning the list off is choosing another provider
  (or none) in Settings → Capabilities, not disabling an extension
  (`capabilities::CapabilityRegistry::resolve`, `.context/work-tracking.md`).
  It also offers snapshots' `hashes`
  (`oxplow:snapshot-hashes`, "Track changes only": no `contents`); a
  chosen `hashes` with bundled disabled falls back to core's `oxplow`.

**Declared needs.** A page or an advisory may declare `needs:
[work_items, snapshots.contents]` — capabilities, or one of a
capability's features (`oxplow_domain::capability::check_need`; an
unknown one is a load error). A page run checks them against what's
active (`ViewContext.active`, a `capabilities::Active` snapshot that
`view_context` fills): unmet, it runs no loader and `PageRun.inactive`
is set (`PageInactive { needs, message }`: "Needs: Work list (choose one
in Settings → Capabilities)."). The page view shows that instead of its
empty state. An advisory whose
needs aren't met doesn't run. A context built without `Services` (param
checks) checks nothing. A capability's none declares every feature (it
takes everything, keeping nothing), so a feature need is met by none and
only a need of the capability itself (`work_items`) isn't. `oxplow-foundation`
declares no `work_items` need: with no list chosen its task pages, hint and
skill just find nothing.

**What an implementation owns.** A provider instance owns its command
namespace (`<id>.*`), offered and run only while it's active
(`capabilities::Active::refusal`, `.context/commands.md` step 0). The
built-ins own nothing of the agent's surface: the work-item tools read
whichever list is active.

## Skills

An extension gives the coding agent skills and slash commands (stable,
`skills:`; `extensions/skills.rs`):

```yaml
skills:
  - { name: work-items, file: skills/work-items/SKILL.md, needs: [work_items] }
  - { name: work-next, kind: command, file: commands/work-next.md }
implementations:
  - { capability: work_items, id: oxplow, entry: "oxplow:tasks", skills: [work-next] }
```

- `name` is lowercase letters, digits and `-`, not one of core's own
  (`oxplow_agent_text::core_text`), declared once; a skill's
  frontmatter `name:` is its `name` and it has a `description:`, a
  command's frontmatter has a `description:`. `kind` is `skill` (the
  default: `<name>/SKILL.md`) or `command` (`/oxplow:<name>`).
- **Offered** (`capabilities::agent_text`): from enabled
  extensions (one with skills declares `agent`), when its `needs:` are met and, if an implementation lists
  it in its `skills:`, while that implementation is the active one. A name
  another extension already took is left out (logged).
- **Installed.** Each spawn writes what's offered into the runtime
  (each harness's `launch`, `crates/oxplow-harnesses`); boot, an
  extension change and a `capability.switched` rewrite the runtimes
  already on disk (`refresh_agent_text` → each harness's `refresh_text`). A skill folder oxplow wrote
  carries a `.oxplow` marker, so one no longer offered is removed and a
  person's own stay; the Claude `commands/` folder is oxplow's whole.
- `oxplow-foundation` gives the `work-items` skill (any work list) and the
  `work-next` command (oxplow's tasks' ready queue).

## Advisories

An extension gives the coding agent guidance — **hints** — with
**advisories** in `extension.yaml` (current). Core owns only the
mechanism; what to say, and when, is SQL in the extension.

```yaml
advisories:
  - id: low-coverage
    on: post-tool-use        # or prompt, or turn-end
    once_per: effort         # effort (default) | thread | session | day | row (needs a `key` column) | turn
    audience: agent          # agent (default) | person
    heading: "# Optional heading line"
    query: |                 # :thread_id, :stream_id, :turn_id, :effort_id
      SELECT '...' AS message FROM v_effort_observation WHERE effort_id = :effort_id ...
```

- **Triggers.** `post-tool-use` runs on each `agent.tool.finished`
  (`advisories.post_tool`), `prompt` on each prompt the person sends, and
  `turn-end` on each `thread.checkpoint` (`advisories.turn_end`, after
  `effort.observe`): evaluated when the turn ends, heard when the agent
  next hears from oxplow. A turn's end is the one point every harness
  reaches, blocked or not.
- **Params.** Every query may use `:thread_id`, `:stream_id`, `:turn_id`
  (NULL at a prompt) and `:effort_id` (NULL when the thread has no effort
  — for an event, the effort it was anchored to; at a turn's end, the one
  holding the turn's end; at a prompt, the open one).
- **Repeat rules.** `once_per: effort` fires once per effort and doesn't
  run without one; `thread` once per thread; `session` once per agent
  session on the thread (the latest turn's harness session; doesn't run
  without one); `day` once per thread per UTC day; `row` once per `key`
  on the thread (a query wanting once per effort per row puts
  `:effort_id` in its key, as `threshold-crossed` does); `turn` whenever
  it has rows. Marks live in `once_mark`, per effort or thread.
- **Audience.** `agent` hints go to the coding agent; `person` hints are
  raised in Alerts ("Hints for you", `apps/desktop/src/hints.ts`) until
  the person dismisses one (`oxplow.hint.dismiss`).
- **Delivery.** Every hit is recorded as a nudge (`v_agent_nudge`, kind
  `<extension>/<id>`, with its `audience`), undelivered. The thread's
  next prompt (`AgentContext::prompt_context`) or tool call
  (`post_tool_context`) takes the agent's undelivered nudges — oxplow's
  own first, oldest first, up to `NUDGE_BUDGET` (2,000) characters, at
  least one — and stamps `delivered_at`; what the budget held waits for
  the next hook. A person's nudge is stamped when they dismiss it. The
  ledger says what was raised and what actually reached its audience.
- **Effectiveness.** Each evaluation is counted (`hint_stat`), and
  `v_hint_stat` puts the counts together per hint and thread: evaluated,
  fired, delivered, held, muted. An agent hint that still fires after
  reaching the agent `MUTE_AFTER` (3) times in its effort (or its thread,
  with none) is muted there — not evaluated again — and the person is
  told once (a `hint-muted` nudge with what it said). A new effort starts
  it fresh.
- `crates/oxplow-app/src/advisories.rs`: `AdvisoryRunner` runs the enabled
  extensions' advisories for one trigger and scope and applies
  `once_per`. Marks are recorded only after every query ran. A failing
  query is logged and skipped.
- **Permission.** An extension's advisories speak into the agent's
  context, so the manifest must declare `agent.write`
  (extensions-permissions.md "Permissions"; the loader refuses it otherwise), approved when the
  extension is deployed. A bundled extension's ship under
  `shipped_permissions`.
- `for_thread(svc, thread, on, cause)` runs them for a thread, reading
  the live extensions (whatever the thread's stream) and
  binding the thread's stream into the scope.
- `oxplow_validate_extension` dry-runs each advisory with every param NULL and
  checks it returns `message` (and `key` for `once_per: row`).
- oxplow-bundled ships hints on outcomes: `large-uncommitted` (turn-end,
  once per effort: an open effort holding 15 or more files — a commit
  landing its work would have closed it) and `landed-in-progress`
  (turn-end, to the person, once per item: an item still in progress
  though a commit landed its work and nothing is open on it); the two
  shared-files hints (turn-end: two open efforts on different threads
  changed one file within `sharedFiles.windowMinutes`; the Alert to the
  person once per pair, and a note to each agent, once per thread; see
  work-tracking.md "Cross-thread ownership"); and two
  metric ones, `metric-deltas` and `threshold-crossed` (see
  [metrics.md](./metrics.md)). **Bundled advisories set no coding
  requirements** (decided 2026-10-07): they say what oxplow observed, never
  how a project should code — no coverage target, no "add tests" — and
  a project that wants one writes its own advisory (the `coverage-target`
  nudge was removed for this).
  Each bundled hint has a fixture in `advisories.rs` that makes it fire
  and cases that keep it quiet (`every_bundled_hint_has_a_fixture_that_fires_it`
  fails when a hint ships without one). A count of `fired = 0` in
  `v_hint_stat` is therefore the project's state, not a broken query:
  `threshold-crossed` needs a metric with a `warnAt`/`failAt` that an effort
  moves past (only the coverage metric ships thresholds, and efforts
  rarely fall under them), `landed-in-progress` needs a commit-closed effort
  whose item is still in progress, and nothing is evaluated while
  oxplow-bundled is disabled, since its policy and hints are the
  extension's. A `coverage-target` row in `v_hint_stat` is residue from
  before that hint was removed. Advisories read
  stored views (`v_effort_metric_delta`, `v_effort_observation`), never the
  engine directly.
- The report-less-run nudge stays in core: it's about collection hygiene,
  not an instrument.

## Agents: the MCP surface

Everything a human can see or build, an agent can see or build too, over
MCP. That includes whatever **extensions** add: an extension's sources,
entities, dimensions, metrics and pages show up in the same generic tools
as core's, and its commands through `run_command`.

**Tools are declarations.** Core builds only the command bus's two tools,
`list_commands` and `run_command`. Every other tool is a command or a page
that asks to be one with `tool: <name>` on its `commands:` entry or its
page file, offered as `<namespace>_<name>` (the extension's `namespace:`):

```yaml
commands:
  - name: sql.query
    summary: Run ONE read-only SQL statement over the published `v_*` datasets. …
    scope: sql.read
    op: query
    tool: query_sql          # offered as oxplow_query_sql
# pages/open-work.yaml
tool: open_work              # offered as <namespace>_open_work
```

- **Typed results.** An operation that declares its result
  (`Op::returns::<T>()`, `T: JsonSchema`) gives its commands an
  `output_schema` (`CommandSpec.output_schema`); its tool lists that as
  MCP `outputSchema` and answers with the result as `structuredContent`
  beside the JSON text, so a client reads it typed. `sql.query` declares
  `SqlQueryResult` (columns, rows, `truncated`, `reads`, `freshness`,
  `generation`, `coverage`). A page tool declares none: what it returns
  depends on its `format`.
- oxplow's own tools are `oxplow-foundation`'s declarations over core's
  read operations (`oxplow_read`, `oxplow_query_sql`, `oxplow_show_page`,
  `oxplow_vcs_log`, …); no tool is special to core.
- A **command tool**'s description, schema and example are its command's,
  and a call runs it as the caller through the bus (validation, policy,
  proposals, audit). It's offered while its extension is enabled and the
  command is offered to the caller (`needs`), so the
  list follows the active implementations as commands do. Its read-only
  hint is its command's access (`View`/`Read`), unless the command
  needs a scope that spends the person's money (`Scope::spends`; today
  `ai.compute`, a model call on their key) or is an operation of one: it
  changes no record but isn't hinted, so the client asks before it runs.
- A **page tool**'s input is one property per param (its label, choices
  and default) plus `format`; a call runs `oxplow.page.run` and returns the
  page's text (`format: "json"`: its loaders' rows). Always read-only.
- A name is `[a-z][a-z0-9_]*`, at most 48 characters; two entries offering
  one name — in one extension, or two under one namespace (oxplow's own
  share `oxplow`) — are refused at load, on each.
- **Typed page tools.** `oxplow-foundation/pages/` holds the pages an
  agent asks for most, each `hidden: true` with a `tool:`: `catalog
  { dataset? }` (every dataset with its columns), `thread_work { thread }` (the
  thread's items by state, with their efforts), `work_next { thread }` (what
  to pick up next; the bundled `/work-next` command calls it) and
  `effort_summary { effort }` (title and summary, item, files, test runs,
  claims, decisions as sections). Params take bare ids (`thr3`, `eff9`).
  Each reads published `v_*` datasets only, so it works with the bundled
  extension off. For history, `oxplow_vcs_log` and `oxplow_diff` are
  commands, not pages.
- Every agent-invokable command stays reachable through `run_command`
  whether or not it's a tool: a tool is a named entry for the ones an
  agent reaches for often, kept few so the list stays readable for weaker
  datasets.
- Listing and dispatch: `oxplow_app::agent_tools` (`offered`, `tools_of`);
  the MCP server (`crates/oxplow-mcp`) lists and resolves them per caller.

**Understanding the semantic layer**

- `oxplow_query_sql`: read-only SQL over the published datasets. `v_dataset` /
  `v_field` / `v_dataset_lineage` describe every view (core `v_*` and
  extension `v_<ext>_<entity>`) with its column docs and inputs;
  `v_dimension`, `v_metric_spec` and `v_measure` list the dimensions,
  metrics and measures, extension-declared ones included, each with its
  scope.
- A metric's numbers are SQL too: `SELECT bucket, MEASURE('<key>') FROM
  metric_grid('day'[, '<dim>'])` — sliced by a dimension, in any bucket.
  Changing metrics is the `metric.*` commands (`run_command`).

**Working with pages**

- `v_page`: every page (`uri`, `extension`, `id`, `title`, `description`,
  `hidden`), with `v_page_param` (its params) and `v_page_answer` (the
  questions it answers); `v_extension` and `v_contribution` give the
  extensions and what they contribute.
- `oxplow_find_page { question?, about? }` (`oxplow.page.find`): the pages
  that answer a question, each with its params. Look here before writing SQL
  for something a person will look at. With `about`, a resource page is
  found when `pages/renderers.rs` offers it for that uri.
- `oxplow.page.renderers { uri, stream_id? }` (through `run_command`): the
  pages that can draw a uri, in Open With's order (`page`, `title`,
  `default`), as the window's `renderers_for` lists them.
- `oxplow_run_page { page | spec, params?, format?, loader?, draft? }`
  (`oxplow.page.run`): **what the UI shows** for those params, as the
  page's text by default, or with `format: "json"` each loader's columns
  and rows (up to 200) and the tree. `spec` is an inline page of the
  agent's own (YAML); it only reads. `draft: true` reads the stream's
  draft of the page's extension.
- `oxplow.page.describe { uri }` (through `run_command`): what a page file
  is made of, its source file, params, loaders and node tree, and which
  page and file each node comes from. `uri` is the file on
  `oxplow:extensions` (the running page; an older `rev`, that deployment's
  copy), `oxplow:temp` or the worktree (`text` is the draft); a file that is
  no page gives no rows.
- `oxplow_show_page` (`oxplow.page.show`): shows the person a page beside
  the conversation, a saved one with params or the agent's own `spec`.
- **Text rendering** (design rule 17, `crates/oxplow-app/src/viz_text.rs`
  and `pages/text.rs`): one renderer per view node kind, and each node
  appears under `[node <id>]` (an embedded page's under `[node
  <embed>/<id>]`), so the agent can point the person at one with
  `oxplow_navigate` and `#node=<id>`. A table or list is a markdown
  table of the displayed columns; a number or markdown node is its first
  cell; a bar chart is its labels and values with the total; a line chart
  pivots to one column per series with each series' total; a treemap
  lists labels by size, largest first; a tree is an indented list; a
  timeline is `at — label (URI)` lines oldest first; a detail is
  `**label**: value` lines; sections are a `### title (URI)` heading per
  row, its `**label**: value` facts, then its body; steps are a numbered `[x]`/`[>]`/`[!]`/`[ ]`
  checklist; hunks are each file's unified diff between its two revisions
  (up to 20 files, 8 KB each). What a node needs beyond its rows — the
  hunks' diffs, a `code` node's files, an `approvals` node's proposals —
  is `viz_text::resolve`d first (`text_of`). Tables stop at 50 rows and
  say how many more; an empty result says the node's `empty` text. It is
  what `oxplow_run_page`, `oxplow_get_open_page` and `copy` return, so an
  agent reads a page the way the person sees it.
- `oxplow_get_open_page(thread_id)`: what the human has open in that
  thread, for any page kind (a work item's, file's or a custom page's
  URI): `pageId` is its identity (the URI without its `#`) and the view
  the `#`, decoded. For a page it adds `pageRun`, the window's own run of
  it as text, with `focus`, the node the person is on. `others` lists the
  page each other window has open there; `tab` reads one of the thread's
  tabs instead (an id `oxplow_list_tabs` listed).
  - The UI reports the active page (`report_open_page`, UI-only) together
    with whatever that page published to `tabs/openPageDetail.ts`.
  - The state is held in memory on the thread runtime
    (`ThreadRuntimeRegistry::open_page`).
  - A hidden-but-mounted page can't overwrite the report, because only
    the active page's detail is sent.
- **Row actions:** right-click a page row → "Ask the Assistant" puts the
  row in the agent's input.
- **What a person can ask:** what pages say they answer
  (`answers:` in a page file; `about` makes it a question on pages for that
  kind of ref; a question is **one line** — a pasted line break is Enter in
  the terminal, so a multi-line one would send itself — checked by
  `answerability::check_pages`; see extensions-sdk.md "Answerability"). It is read as
  **`v_page_answer`** (with `v_page`), beside **`v_config_key`**
  (`config_key`: project.yaml's keys with their doc and value schema), with
  `oxplow_query_sql`; there is no catalog RPC and no separate prompt list
  (`intent.prompts`, `v_prompt` and `prompt_catalog` are gone).
  - The UI: the **Catalog** page (oxplow-foundation's `catalog`, with
    `catalog-prompts` — grouped by the page's category (else its
    extension), with an Ask row action on `oxplow.agent.draft` — and
    `catalog-config-keys`, with Open Settings as a page action on
    `oxplow.window.navigate`); the nav bar ☰'s Ask (suggested prompts for
    the page's resource, `usePromptCatalog` over `v_page_answer`); and
    `EmptyState`'s prompts (usability.md → "Empty states").
- **No launcher entries.** The manifest has no `launcher:` list. A page is
  a file in `pages/`; a command — one that runs another with fixed input,
  or puts a prompt in the agent's input (over `agent_input.write`
  `draft`) — is a command the extension declares with a `ui`, offered like
  oxplow's own (`commandOffers`). The launcher lists the pages that have
  a `category`.
- **Page actions:** `actions:` are commands. An agent presses one with
  `oxplow_run_page_action` as the page acting for it (see "Actions are
  commands"). A command the agent may not run, or one that needs a
  person's confirmation, is refused.

**Building**

- Agents author extensions and pages **by editing files** under
  `.oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  write guard. Editing a source changes nothing until
  `oxplow.extension.deploy { name }` (extensions-deploy.md "Sources and deployments"); the
  authoring tools read the source, so the agent checks its work first.
- **No scaffold tools (decided).** A scaffold tool would write project
  files outside the write guard and the caller's worktree. The skill carries the templates instead, and humans have
  Save as Page in Explore Data. The same rule holds for metrics:
  the `oxplow.metric.scaffold` command returns the metric extension's files for
  the agent to write itself.
- `oxplow_validate_extension(name)` returns load errors, schema errors and a dry
  run of every page (`pages::checks`: each SQL loader runs once, a `columns`
  key the query does not return is an error, `layout_warnings` flags a
  layout that is likely wrong), so the agent can check its work without
  the UI. `oxplow extension check` is the same report.
- `v_extension`, `v_collector_state`, `v_collector_credential`
  (set or not, never the value), the `oxplow.collector.sync` command (never grants).
- **Worktree streams: preview, don't run.** Collected data is
  project-wide (one `ext__<ext>__<entity>` table), so a run (the
  `oxplow.collector.sync` command over `CollectorRunner::sync`, from any caller)
  always runs the **live** copy. An agent
  writing a collector tests it with `$OXPLOW_BIN extension test <name>` (a
  throwaway, through its examples); MCP
  `oxplow_preview_collector(owner, id, stream_id)` dry-runs what is
  deployed against that stream's files and data: `collector_runner::
  preview_collector` runs the deployed entity collector through the same
  `produce` step (same permissions: an exec collector needs
  `process.run`, and so does a Starlark one that calls a
  dataset need `ai`; other derived collectors need neither) and returns the coerced
  rows per entity (first 50, plus totals), storing nothing and recording
  no run. A **fact collector** (`facts:` declared) previews through the fact
  engine instead (`MetricsService::preview_fact_collector_in`, the same
  runner `oxplow extension test` uses): it runs that worktree's script over
  the stream's latest snapshot and returns `{ owner, id, facts }`, the
  facts it would record, recording nothing. One that reads `input` rows is
  previewed through an example, and a program (`exec`) is never previewed.

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-agent-text/assets`)
teaches the format, the `v_*` contract and the loop "read `v_dataset` →
oxplow_query_sql → write files → oxplow_validate_extension → oxplow_run_page".
Its page references (`page-format.md`, `loaders.md`, `view-nodes.md`,
`recipes.md`, …) describe the page file. Add to Context on a page puts
the page's URI in the thread's basket.

**Getting newcomers there.** "New Page with Your Agent…"
(`oxplow.page.draft_with_agent`, a Tasks command in the launcher; needs a
thread) puts a starter prompt into the agent's input, unsent. It never
sends; the person finishes the sentence.
