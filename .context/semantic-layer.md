# Semantic layer

This doc covers the layer that **all oxplow data** flows through: the
sources that produce data, the dimensions that slice it, the metrics that
aggregate it, and the read-only SQL contract (`v_*`) that views, extensions
and agents query.

> **Status: partly built (epic tsk275).**
> - **Current:** the `v_*` read contract for core data, plus `query_sql` and
>   the catalog as models (`v_model*`), and the **fact substrate**
>   (`measure`, `dimension`, `metric_spec`, `metric_capture`, `fact`, cube),
>   documented in [metrics.md](./metrics.md).
> - **Current (tsk289):** extension `exec` sources that bring external
>   entities in (see "User and extension sources").
> - **Current:** decisions and claims (`v_decision`, `v_claim`, MCP
>   `record_decision` / `record_claim`), the agent-activity views, stored
>   change analysis (`v_change*`), and extension-declared measures,
>   metrics and fact collectors (tsk311, see extensions.md).
> - **Current (tsk277):** git, LSP-diagnostic and test-run views
>   (`v_commit*`, `v_branch`, `v_diagnostic`, `v_test_run`, `v_test_case`);
>   spine dimensions and time buckets on fact metrics; entity metrics and
>   entity (expression/join) dimensions (tsk322, see "Entity metrics" in
>   metrics.md).
> - **Current (tsk323):** starlark/jq derived sources and `sync: upsert`
>   with tombstones.
> - **Current (tsk324):** the exec-source `network` allowlist, enforced
>   on macOS.
> - **Current (tsk325):** Settings → Data: every entity with provider and
>   row count (IPC `list_data_entities`, UI-only), and the source rows.
> - **Current (P4, epic tsk484):** every published view is a **model**
>   (core files and extensions' `models:`, compiled with lineage,
>   contracts, versions and tests; the registry `v_model*` is the
>   catalog); the read contract is **enforced** by the authorizer;
>   metrics read in SQL through `metric_grid()` / `MEASURE()` and
>   `metric_findings()`; results carry their `reads` and `freshness`, and
>   views re-run when a model they read changed; the code models
>   `v_function` / `v_file_metric` over `v_tree_fact`.
>
> When a piece ships, move it from "target" to "current" here, in the same
> commit.

## Why

Oxplow's UI grew one bespoke page per question: metrics, dashboards,
change analysis, usage. Each page has its own IPC calls and its own opinion
of what matters. The direction ([architecture.md](./architecture.md) →
"Skeleton + semantic layer") is to stop hand-building answers and instead:

- ship **the data** as a stable, queryable layer;
- ship **common sources** of that data;
- let users and their agents build the **lenses** they actually want
  ([extensions.md](./extensions.md)).

A lens can only be as good as the data it can reach. So **nothing may be
reachable only through a bespoke IPC**. If core has it, the semantic layer
exposes it.

## The three pluggable parts

### 1. Sources produce data

A **source** emits:

- **entities**: typed rows with a key and relations (a task, a commit, an
  LSP diagnostic, a GitHub PR); and/or
- **facts**: atomic measurements on the existing fact substrate (a
  function's complexity, a test case's outcome, a token count).

Core ships built-in sources. Extensions declare more with the same format.
A source runs on a `schedule`: `every <dur>`, `on-snapshot`,
`on-effort-complete` or `manual`. The existing `CollectorRegistry` runtimes
(`BuiltinRust`, `Jaq`, `Starlark`, `Exec`, in `crates/oxplow-collect-plugin`)
become the source runtimes.

### 2. Dimensions slice data

A **dimension** is a SQL expression or a join over an entity. Examples:
`task.priority`, `file → package`, `file → owner via CODEOWNERS`,
`pr.author`, `issue.cycle`. A dimension can be declared anywhere (core,
project config, any extension). It applies to every metric whose entity has
the columns it needs. So an extension can add a dimension over data that a
different source owns.

Fact dimensions are keys in `fact.dims_json` (plus the capture-spine
dims), resolved in `metric_engine.rs::dim_value_cached`. **Entity
dimensions** (current, tsk322) are `dimensions:` entries with `entity`,
`expr` and an optional `join`. They slice entity metrics over the same view.
Today a dimension applies to metrics over its own view only, not to every
entity that has the columns.

### 3. Metrics aggregate data

A **metric** is an aggregation over an entity **or** a fact set, sliced by
dimensions:

- `count(v_task) where status = 'done'`
- `median(v_github_pr.merged_at - opened_at)`
- `avg(fact oxplow.coverage)`

Current (tsk322): a `metrics:` entry with `entity` (plus `where`, `time`,
`value`, `aggregation`) aggregates a view. `time` makes it an event metric,
computed live. Without `time` it is a state metric, captured as facts. Both
kinds go through the same `metric_spec` catalog and read paths (Metrics
page, Metric Detail, dashboards, MCP). Mechanics: "Entity metrics" in
metrics.md.

## Shipped sources (core)

What core publishes, by where it comes from. Each row names its models
(`v_<name>`); `v_model` lists them all.

| Source | Models | Facts |
|---|---|---|
| work | `stream`, `thread`, `task`, `task_note`, `task_link`, `effort`, `effort_file`, `decision`, `claim`, `context_read`, `struggle` | cycle time, steering, lifecycle |
| knowledge | `wiki_page`, `comment` | freshness |
| git | `commit`, `commit_file`, `commit_task`, `branch`, and change analysis (`change`, `change_file`, `change_function`, `change_import`, `change_duplicate`, `change_test_file`) | churn |
| snapshots | `snapshot`, `snapshot_op` | — |
| lsp | `diagnostic` | diagnostic counts by severity |
| tests & coverage | `test_run`, `test_case` | coverage, pass/fail |
| code metrics | `function`, `file_metric` over `tree_fact` (the current tree of every per-path measure — the engine's fold) | complexity, length, params, TODOs, doc coverage |
| agent | `agent_turn`, `tool_call`, `token_usage`, `agent_nudge`, and agent events (hooks, sessions) as `v_event` `agent.*` | tokens, struggle |
| ai | `ai_call` | tokens, latency |
| usage | `page_visit` | — |
| metrics | `measure`, `dimension`, `metric_spec`, `metric_catalog`, `capture`, `fact`, `effort_metric_delta`; series through `metric_grid()`, offenders through `metric_findings()` | — |
| the log and the registry | `event`, `event_content`, `event_checkpoint`, `event_dead_letter`; `model`, `model_column`, `model_lineage`, `model_test` | — |

Live-only for now — read over LSP or git when asked, not stored, so no
model: blame lines, and LSP symbols and references. A collector that
stores them would add their models. Decisions, claims, context reads and
struggle signals are what a person reviewing agent work needs most —
**exceptions and decisions**, not trends and totals.

## Models (P4.2)

A published view is a **model**: one `SELECT` in a file, reading only
through `ref('<model>')` and `source('<table>')`, compiled to a view at
every open (`crates/oxplow-db/src/models.rs`, called from
`migrate_and_compile` after the migrations). Core models live in
`crates/oxplow-db/models/`: `<name>.sql` plus an entry in `models.yaml`
(`name`, `version`, `description`, `columns` — the contract, with each
column's SQLite type and doc — and `tests`); the build script embeds every
file there, and view `v_<name>` publishes it. The compiler, per owner, in
one transaction:

- resolves `ref()` / `source()` with the tokenizer — an unknown model or
  table, or a non-literal argument, is an error at `file:line:col`;
- orders the models so each is created after what it reads (a cycle is an
  error naming each model and where it wrote its first `ref()`);
- drops the owner's previous views and creates the new ones;
- checks **lineage**: everything the authorizer reports the view reading
  directly (`ReadSession::direct_inputs`) must be a declared `ref()` or
  `source()` — reading a table without `source()` fails. The converse
  isn't checked: each declared input is a call in the SQL, substituted
  into it, so it's read by construction (and SQLite can't always say so —
  a model made only of CTEs reports its reads under the CTEs' names, P7.B5);
- checks the **contract**: the view's `PRAGMA table_info` must equal the
  declared columns, and `model_contract` holds what each `(view, version)`
  promised — a changed contract at the same version fails naming the
  column ("bump its version"). A column's **doc is part of the contract**:
  rewording one is a bump too, else every database that recorded the
  version refuses to open (`ai_result` went v3 for a reworded `caller` doc
  in P7.B3's review, `claim` and `decision` v2 for P7.C4's; the test
  `a_database_holding_an_earlier_published_contract_still_opens` replays
  such a database). Because that check needs a database that recorded
  the version, the core models' contracts are also **pinned in a golden**,
  `crates/oxplow-db/fixtures/model_contracts.json` (`{ name: { version:
  columns } }`, every version published): `every_core_model_contract_is_
  pinned_at_its_version` fails in CI on a change at a pinned version, and
  `OXPLOW_BLESS=1` pins a new one (earlier versions stay);
- records `model` (view, name, owner, version, description, compiled
  SQL), `model_input` (`ref` | `source`) — V105.

**Who owns a view** is the registry's answer (P4.9, V109): `model.kind`
is `sql` (compiled from a model file at every open — `drop_all` drops
only these) or `entity` (an extension's synced entity, created by
`collector_store::write_entity` when its collector runs and kept across
opens). An entity view is registered with its extension as owner and its
`ext__<ext>__<entity>` table as a `source` input, so lineage and
subscriptions see it. `write_entity` replaces a view only when the
registry says it is that extension's entity — core's or another
extension's is refused naming the owner — and recreates a missing view
without dropping its table. `drop_extension` drops the extension's views
by the registry. V109 registered the entity views synced before it.

**Extension models** (P4.9, `crates/oxplow-app/src/extension_models.rs`).
An extension declares `models:` in `extension.yaml` — each entry the same
`ModelDecl` as `models.yaml` (name, version, description, columns,
tests) — with its SQL in `models/<name>.sql` (`models::join_sources`;
a declaration without its file, or a file without its declaration, is an
extension error). Each publishes `v_<extension>_<name>` (dashes as
underscores — the entity views' formula). Inside one:
- `ref('x')` is the extension's own model or entity `x`, else the core
  model `x`; `ref('<ext>/<name>')` is another extension's, explicitly;
- `source()` reads only the extension's own `ext__<ext>__*` tables —
  oxplow's data comes through `ref()`.

A check (`models::check_extensions`, P7.C6) runs the same pass with temp
views, standing an empty view in for each declared entity that hasn't
synced, and hands back what it created for the check's other queries to
read (`SqlQuery::temp_views`) — see extensions.md "The SDK".

`models::compile_extensions` compiles every enabled extension's models in
one pass after the core ones: the last pass's extension views go, the
rest publish in dependency order, each in a savepoint — a model that
fails (resolution, lineage, contract, a name that belongs to core or an
entity), and every model reading it, is left out and reported for its
extension; a cycle is reported for each model in it. Only the primary
worktree's extensions compile (views are project-wide, like source
data). `ExtensionModelsService` runs a pass at boot and after an edit
under `oxplow/extensions/`, a config change, or a registry change (a
source syncing a new entity); its fingerprint — the enabled extensions'
model sources plus the registered entity views — makes a pass over the
same inputs, including the one its own registry writes set off, a no-op.
**Versions** (core and extensions alike): a changed contract at the same
version is refused, so a breaking change bumps `version` — and may keep
the old one published under `deprecated: [{ version, file, until }]`.
The kept version compiles from its own SQL file (beside the model's) as
`<view>_v<version>`, held to the contract that version published
(`model_contract`), until `until` (`YYYY-MM-DD`). Past its date, or
for a version that never published, it isn't kept: an extension hears
why in its errors; core just drops it (a date can't fail boot).

After publishing, each extension model's declared tests run
(`run_tests`, results in `model_test` / `v_model_test`; a relationship's
`to` resolves like a `ref()`); a failing or erroring test is reported for
its extension, and the view stays published. The errors are the
extension's health: `list_extensions` (IPC and MCP) merges them into
`errors`; nothing fails boot. `drop_all` removes the
extension models' registry rows with their views, so between an open and
the first pass the registry lists none.

Declared tests (`not_null`, `unique`, `accepted_values`,
`relationships`, `sql` returning failing rows) run through
`models::run_tests` and record `model_test` (`passed` / `failed` /
`error`). A core model's failing test is recorded, never a boot failure: a
data problem in someone's database mustn't make it unopenable
(`every_core_model_passes_its_tests_on_an_empty_database` keeps the
declarations honest). **Built (P4.2, tsk487):** all 49 core views are
models (`the_core_models_reproduce_the_views_the_migrations_made` proves
each one's columns and types equal the migration view it replaced),
`v_model*` are models over the registry, and `CATALOG` /
`SemanticLayer::describe_schema` are gone. Views are dropped before the
migrations and compiled after them at every writable open; a read-only
open (`oxplow plugin check`) uses what the last open compiled.

### Materialized models (P7.B2)

A model may declare **`materialize: on_change`** (`ModelDecl.materialize`;
core's `models.yaml` and an extension's `models:` alike). It compiles and
is checked exactly like any other — its SELECT against lineage and
contract, `model_input` recording what it reads — and then publishes as a
view over its own table, **`m_<view>`** (`models::materialized_table`),
which has the contract's columns and declared types (not STRICT, so a
computed, untyped column stays untyped, as the view reports it). The
table keeps its rows across opens (they are the last recompute's); a
changed contract recreates it empty for the first recompute to fill.
`model.materialize` (V131) records the policy. A model never
`source()`s an `m_*` table — its own or another's ("a model may not read
itself"); it reads a materialized model through `ref()`. A query's own
SQL can't read one either: the read contract refuses it, naming the
model over it. `plugin check` (the read-only `Pass::Check`) creates the
temp view only, never the table; a table no published model reads goes
after the extensions' pass (`drop_orphaned_tables`).

**Recomputing** is the asset runner's ("Assets"): `Assets::sync_models`
keeps one `SqlModelMaterializer` per materialized model — its inputs are
its `model_input` followed through live models down to tables (and to a
materialized input's own table) — re-run when the registry (`model`,
`model_input`) changes. A recompute refills the table, whole, in one
transaction (`DELETE`, then `INSERT … SELECT`), after its inputs have
been quiet for the coalesce window, and is recorded in `asset_state`.
**Lineage for subscriptions** routes through the table: a materialized
model changes when `m_<view>` is refilled, not when its inputs move, so
`ModelsChanged` names it and its readers once per recompute. Its
watermark is therefore when it was computed; `v_model` (v3) adds
`materialize`, `computed_at` and `events_to` (from `asset_state`).
**Not yet:** `interval:` freshness, incremental recompute, and keys in a
contract.

## Metrics in SQL (P4.5)

A query reads metrics as columns of a grid:

```sql
SELECT bucket, zone, MEASURE('oxplow.coverage.abs_pct')
FROM metric_grid('day', 'zone')
```

- `metric_grid('<day|week|month>'[, '<dimension>'])` — exactly one per
  query; its rows are `bucket` (the bucket's start date, `YYYY-MM-DD`),
  the dimension column when given, and one column per `MEASURE('<metric
  key>')` the query names (keys in `v_metric_spec`, dimensions in
  `v_dimension`).
- **The engine stays the one authority** on what a metric means: each
  `MEASURE` is `MetricEngine::series_for_spec_read` for that spec, bucketed
  and grouped — its aggregation, temporal fold, filter, scale and the cube
  all apply, so a week of a complete measure is its last capture, not a
  SQL sum. SQL then filters, joins and aggregates the grid like any table.
- **How** (`crates/oxplow-app/src/metric_grid.rs`, in the SQL gateway):
  the tokenizer finds the calls; every series is read **before** any
  connection is taken (the engine holds its own pool permits — it runs on
  the one-connection in-memory database); the points become
  `SqlQuery.temp` — `temp."metric_grid_1"` — created and filled on the
  query's connection before the authorizer and `query_only` go on and
  dropped after, on every path; `metric_grid(…)` and each `MEASURE(…)` are
  rewritten to the table and its `"measure:<key>"` columns. The result's
  `reads.measures` lists the measures behind it (what a subscription
  watches, P4.6). `check` resolves the metrics and compiles against an
  empty grid.
- The series are scoped to `SqlQuery.stream` — a lens passes its
  `:stream_id`.
- **`metric_findings('<key>'[, <capture id>])`** (P4.8,
  `crates/oxplow-app/src/metric_findings.rs`) is the same mechanism for the
  located items behind a metric: a table of `subject_kind, subject_ref,
  path, line, value, severity, rule, message, branch, captured_at`, one row
  per fact the metric's filter keeps. Without a capture it is the metric's
  **current** state (`MetricEngine::current_facts`: the tree fold for a
  per-path measure, the latest capture per stream and producer for a
  complete one — a fixed item disappears — every fact for an additive
  one); with one, exactly that recording's. `severity` is the fact's own,
  else the value against the metric's `warn_at` / `fail_at` in its
  `direction`. It becomes `temp."metric_findings_1"`; a query may use it
  beside a `metric_grid()` (the gateway plans each over the SQL the other
  rewrote). A formula or entity metric has no findings (an error names
  it). Replaces `v_fact` for "what's wrong now", which a raw fact table
  can't say: an old fact of a since-fixed item is still a fact.
- `metric_grid('capture'[, dim])` keeps **one row per capture**: `bucket`
  is the capture's time and a `capture_id` column joins `v_capture` for its
  branch, provenance and git version — what the metric pages read for
  their recordings (P4.7).
- Errors name the `MEASURE`: an unknown key, a dimension the metric can't
  be grouped by, a formula metric (refused, not empty), and a gateway with
  no engine.
- `grid_rows_equal_the_engine_series` holds the grid to the engine's
  series for every metric, by bucket and by dimension.

## Subscriptions (P4.6)

What a query read is what it subscribes to.

- **Tables:** every pooled connection gets a preupdate, a commit and a
  rollback hook at init (`crates/oxplow-db/src/changes.rs`). The preupdate
  hook — not the plain update hook — also fires for WITHOUT ROWID tables,
  and while it is set SQLite skips the truncate shortcut, so a bare
  `DELETE FROM t` reports. A table joins the committed set only when its
  transaction commits (a rollback forgets it); temp tables never do. After
  each `Database::call` / `call_mut` / `transaction` / `read` the committed
  set is published — after the commit, so a subscriber that reads on
  hearing it sees the change: `Database::subscribe_changes()`.
- **Models:** `crates/oxplow-app/src/models_changed.rs` follows
  `model_input` from those tables to every model that reads them, directly
  or through other models (reloading the lineage when `model_input`
  itself changes, and treating a lagged channel as "everything changed"),
  stamps each one's in-memory watermark (`ModelWatermarks`, derivable, so
  not persisted) and emits `OxplowEvent::ModelsChanged { models }`.
- **Results:** `SqlQueryResult.freshness` is the watermark of each model
  the query read that changed since the app started.
- **Metric samples:** the same loop announces
  `MetricSamplesChanged { stream_id, measures }` when a commit touches
  `metric_capture` or `fact` (`CaptureListener`: the measures of the facts
  that landed since it last read, per stream; a capture with no facts is
  announced with none, fail-open). It is the event's one emitter (P7.B1).
  A `metric_grid()` query's `reads.measures` pairs with it: facts land on
  every OTLP burst, and the measure scope keeps a metric tile quiet unless
  its own measures moved (tsk198).
- **Assets:** the same loop tells the asset runner which tables changed
  (see "Assets").
- **The UI** (`src/lens/lensRerun.ts`): every lens host — the lens page,
  slots, dashboard lens tiles, the rail's alerts and the explorer — keeps
  its last run's `reads` and re-runs through `useRerunOnChange` when
  `modelsChanged` names a model it read, `metricSamplesChanged` names one
  of its measures (an empty list is "unknown", so it re-runs), or a lens
  definition under `oxplow/extensions/` changed. Nothing else re-runs it:
  the old everything-but-two deny-list (`shouldRerunLens`) and its 750 ms
  debounce are gone; a burst of commits coalesces into one re-run
  (100 ms).

## Assets (P7.B1)

Derived data whose inputs are **tables** is an asset
(`crates/oxplow-app/src/assets.rs`): a `Materializer` names its asset and
input tables and recomputes it; `Services.assets` (an `Assets` runner) is
told by the change loop which tables each commit touched, marks the
materializers reading them dirty, and recomputes each once its inputs have
been quiet for `COALESCE` (1 s) — a sweep's burst of writes is one
recompute. Each recompute is recorded in `asset_state` (V130) and read as
**`v_asset`** (`asset`, `computed_at`, `events_to` — the log's highest
seq as it began — `snapshot_id`, `elapsed_ms`): an asset's freshness and
provenance. A registered asset builds once at registration (its
backfill); a failed recompute is logged and the next change retries.

Derived data whose inputs include the world **outside** the tables (a
snapshot's blobs, the VCS tree, a program, a provider) is ingestion
instead — a collector or a pump consumer, at-least-once and checkpointed
(target-architecture.md §8.2).

The metric cube is the first asset (inputs `metric_capture`, `fact`): see
[metrics.md](./metrics.md). What a burst costs it is in
[performance.md](./performance.md).

## The `v_*` contract (current)

Every shipped entity is exposed as a stable **read-only SQL view**. They
are the **versioned contract**: lenses, extensions and agents read these,
never the physical tables, which stay internal and free to change.

**Shipped today** — each a core model, `crates/oxplow-db/models/<name>.sql`
(see "Models"); the version history in the parentheses is the migrations
that made them before P4.2. `v_model`, `v_model_column`, `v_model_lineage`
and `v_model_test` are the catalog of all of them:

| View | What it is |
|---|---|
| `v_stream` | streams (worktrees) |
| `v_thread` | threads within a stream |
| `v_task` | tasks, excluding deleted; carries the thread's `stream_id` |
| `v_knowledge_touch` | which threads wrote which knowledge pages, and when each last did (`page`, `thread_id`, `last_seen_at`; the rail's Finished section, P6.E1b) |
| `v_knowledge_page` / `v_knowledge_ref` | knowledge pages with their outbound refs and `stale_ref_count`, and each page's file refs with their pin and `stale` (P5.C4; knowledge.md) |
| `v_work_item` | work items from every provider (`ref`, `provider`, canonical `state`, `native_state`, `native` JSON, `parent_ref`), excluding deleted; oxplow's tasks are `work_item:oxplow:tsk<n>` (V115, P5.C1; data-model.md) |
| `v_effort` | bracketed spans of work on a work item (`work_item` ref; `task_id` derived for oxplow tasks, V100) |
| `v_comment` | comment threads, with first-message `body` and `message_count` |
| `v_wiki_page` | wiki pages (excerpt; full body is on disk) |
| `v_snapshot` | worktree snapshots, with `tree_hash` (whole-tree identity, V96) |
| `v_snapshot_op` | the snapshot operation log: one row per take (a new snapshot or an unchanged tree) with `parent_snapshot_id`, `trigger`, thread / turn / effort anchors, `elapsed_ms`, `budget_ms`, `over_budget` (V97) |
| `v_measure` | fact-type catalog |
| `v_capture` | the scan/run that produced facts |
| `v_fact` | atomic measurements, joined to `measure_key` and capture context |
| `v_effort_file` | files each effort touched, with change kind and the effort's `work_item` (V74, V100) |
| `v_task_note` | task / thread notes (V74) |
| `v_task_link` | typed links between tasks (V74) |
| `v_agent_turn` | human prompt → agent answer, per thread (V74), with the snapshots the turn started and ended at (`start_snapshot_id`, `snapshot_id`: what the turn changed; V98/V99) |
| `v_token_usage` | model tokens per thread / effort / model, with each turn's prompt (V74, `prompt` V82) |
| `v_page_visit` | pages the human opened, and for how long (V74) |
| `v_event` | the event log: every activity and state change, oldest first by `seq`, with anchors, subject refs and payload (V94; see [data-model.md](./data-model.md) "event_log"); `payload_expired_at` since V102) |
| `v_event_content` | large event bodies (tool input/output, prompts) by content hash, without the bytes (V102) |
| `v_event_dead_letter` | events a consumer failed on, parked with the error; `pending` ones need `retry_dead_letter` / `discard_dead_letter` (V94) |
| `v_event_checkpoint` | how far each event consumer has read (V94) |
| `v_decision` | forks the agent resolved (question, choice, alternatives, confidence, why). `provenance`: `recorded` via MCP `record_decision` (V76), or `inferred` when the effort closed (V79) — a recorded `extract` computation on the `main` role (`AiCompute`, so an unchanged effort doesn't call again) — and a reviewer's verdict on an inferred one: `confirmed` or `dismissed` (V132, `effort.confirm_decision` / `effort.dismiss_decision`; a re-inference replaces only the still-`inferred` ones) |
| `v_claim` | agent claims ("tests pass") with `verified` (cited evidence — `reviewer` when a person verified it with `effort.verify_claim` — or a `tests_pass` claim whose effort's **latest** `v_test_run` — its own or one claimed through attribution — has `failed = 0 AND total > 0`) (V76; latest-run rule V91, tsk366) |
| `v_tool_call` | every agent tool call: the `tool_call.project` pump consumer's projection of `agent.tool.finished` (one row per event; the ingest parses the payload with `oxplow-app/src/tool_calls.rs`; paths worktree-relative; Bash `ok` is NULL when Claude reports no exit code) (V77; `turn_id`, `event_id` since V102) |
| `v_context_read` | `Read`s of `.context/*.md` (V77) |
| `v_struggle` | per effort: a file edited 5+ times, or 3+ failed commands (V77) |
| `v_ai_call` | oxplow's own model calls: role, provider, model, caller, tokens, latency, ok/error, and a recorded computation's `input_hash` (V78, V118; see [ai-providers.md](./ai-providers.md)) |
| `v_ai_result` | recorded AI computations (`classify` / `score` / `summarize` / `extract`), keyed by input hash, model and prompt version, with the output, tokens and the computing call (V118; [ai-providers.md](./ai-providers.md) "Recorded computations") |
| `v_metric_spec` | metric definitions: aggregation, direction, target / warn / fail (enabled lives in project.yaml) (V80) |
| `v_agent_nudge` | guidance oxplow sent the agent mid-effort (V80; `turn_id`, `delivered_at` since V102) |
| `v_code_quality_scan`, `v_code_quality_finding` | code-quality scans and their findings (duplicate blocks, with the peer in `extra_json`) (V80) |
| `v_dashboard`, `v_dashboard_item` | user dashboards and their tiles (V80) |
| `v_effort_metric_delta` | per effort, how each metric moved (baseline → current, `crossing`). **Stored, not a live query:** the metric engine computes it (`CollectionService::refresh_effort_evidence`) and `effort_evidence.rs` refreshes it on `effort.finished` (the `effort.evidence` pump consumer) and for every open effort as the **asset** `effort_evidence` (`OpenEffortEvidence`, P7.B6: its inputs `metric_capture`, `fact`, `effort_attribution`, `agent_token_usage`; its own tables aren't, so it can't loop); readers hear the rows move as `ModelsChanged` (V80) |
| `v_effort_observation` | per effort, test runs / diff coverage / analysis rebuilt from its claimed captures; refreshed the same way (V80) |
| `v_change`, `v_change_file`, `v_change_function`, `v_change_import`, `v_change_duplicate` | stored change analysis (see "Change analysis" below) (V81) |
| `v_commit`, `v_commit_file`, `v_commit_task`, `v_branch`, `v_tag` | history, branches and tags (V85, V113). The commit indexer (`commit_indexer.rs`, run at boot and on each ref move, `RefMoves`) stores the commits reachable from **every stream's** head — new history whole, up to a 5000-commit horizon (`IndexDepth`) — through the VCS capability (`Vcs::log`/`revision`, `.context/vcs.md`) — with their files (first-parent diff) as it projects them into `page_ref`; `v_commit.parents` (JSON, v2) lets a stream's history be a recursive read from its branch's head. `v_commit_task` reads the indexer's task-mention edges. `refresh_refs` restates `v_branch` (`is_default`, v2) and `v_tag` from `Vcs::branches`/`tags` and maps each local branch to the stream checked out on it. The desktop's history panel, dashboard lists, branch picker and new-stream form read these models (`apps/desktop/src/vcsHistory.ts`) |
| `v_test_case_stat` | each test's summary per stream, branch and producer (V139, tsk733): last status and duration, runs, failures, flips, last failed / passed, max and mean duration — updated with every run, the home of per-test history since a run writes change-only per-case facts |
| `v_test_run`, `v_test_case` | test runs (V87), views only. A run IS its `metric_capture` (producer `tests`, or `test-run` for a run that measured nothing), and both views read its verbatim `test-detail` payload in `detail_json`: counts from the payload, cases by `json_each` over `suites[].cases[]`. Cases come from the payload, not the `oxplow.test_case` facts, because those are skipped while every tests metric is disabled. `effort_id` is the effort whose ledger claims `run:<id>` (kind `run`), else the capture's own. A run that only reported counts (MCP `record_test_run`) has no cases |
| `v_thread_answer` | the lenses an agent showed on a thread (`show_lens`, V124, P6.C1): `ref` `answer:<id>`, `thread_id`, `turn_id` / `effort_id` open when it was shown, `title`, either `lens` (an existing lens id) or `spec` (the `LensSpec` as JSON), `params`, `created_at`, and `kept_lens` once someone kept it (extensions.md → "Thread answers") |
| `v_diagnostic` | what the language servers have published, right now (V86). `lsp_diagnostics.rs` subscribes to the LSP session broadcast and replaces a file's rows per `(stream, language, path)` on each `textDocument/publishDiagnostics` (paths repo-relative, positions 1-based, a URI outside the worktree dropped). Live state: the table is cleared at boot and a server's rows when it restarts, crashes or stops. Only files a server has published appear (usually the open ones, not the whole repo). A `DiagnosticsChanged` event goes out at most every 500 ms per stream, from the first change, so a server that publishes continuously can't starve it; with it, `code.diagnostics.changed@1` is logged per changed file (P5.C5) |
| `v_symbol` / `v_symbol_capture` | the symbols the running language servers report for each stream's files (V117, P5.C6) — the current tree, restated per changed file at each snapshot by the `symbols.collect` pump consumer (`ref` `symbol:<path>/<name>@snap:<id>`, nested names as container paths, unique — a repeated name path numbered `~2`; the name's `line`/`col` and the whole symbol's `start_*`..`end_*`, v2), and what each snapshot's collection covered (collected, failed (the server errored or timed out; V120 `files_failed`), over the `symbolsMaxFilesPerSnapshot` bound — which counts attempts — no running server). Only languages with a running server are covered: the collector never starts one (lsp.md) |

Still target: the rest of the shipped-sources table above (tsk327).

**Column docs live with the model, not here.** `models.yaml` documents
every column, and the registry is the catalog: `v_model` +
`v_model_column`, read through `query_sql` (extension entities too —
their contract is their declared columns; tsk517). The compile at
open fails if a view's columns and its declared ones disagree, in name,
type or order. So changing a view means:

1. editing its `<name>.sql` and its `models.yaml` entry — and, when the
   columns change, bumping its `version` (a changed contract at the same
   version is refused);
2. noting the change here if it breaks readers (removed or renamed
   columns).

An extension entity `<entity>` owned by extension `<ext>` is exposed as
`v_<ext>_<entity>` (see "User and extension sources").

### Querying (current)

- `query_sql(sql, params?, limit?)` over IPC (`querySql` in `api.ts`) and
  MCP. **Every query goes through the SQL gateway** (P4.1, tsk486):
  `Services.sql`, `crates/oxplow-app/src/sql_gateway.rs` — MCP and IPC
  `query_sql`, lenses, advisories, entity metrics, extension checks
  (`validate_extension`, `oxplow plugin check`) and source inputs alike.
  It takes one `SqlQuery { sql, params (positional | named), limit,
  timeout }`; `check(sql)` prepares without running. Mechanics
  (`SemanticLayer` in `crates/oxplow-db/src/semantic_layer.rs`):
  - First gate: the tokenizer (`crates/oxplow-db/src/sql_tokens.rs`, the
    one tokenizer: strings, quoted identifiers, comments, parameters,
    calls) — exactly one `SELECT` or `WITH`, a trailing `;` aside.
  - Real gate: `Statement::readonly()` rejects anything that writes
    (including `WITH … DELETE`).
  - **The authorizer enforces the read contract** (P4.3, tsk488; it only
    recorded in P4.1). The query's own SQL — a CTE body included — may
    read models (views) and `temp.*` tables; a stored table is refused as
    "`task` is a physical table, not a published model; read v_task",
    naming the models whose `source()` it is (from `model_input`), and
    never with SQLite's "no such table" (which `explain_unsynced` reads as
    an unsynced source). `count(*)` over a view is allowed (SQLite reports
    its base table at top level after the view's own reads). **A read's
    accessor is the view *or CTE* it happened in** — SQLite names a CTE,
    not the view around it — so the session takes the statement's own
    CTE names (`sql_tokens::cte_names`): a read in one of them is the
    statement's and is checked; a read in any other view or CTE is a
    view's (a model built on CTEs, like `tree_fact`, reads fine). A
    statement CTE sharing a name with a view's CTE refuses more, never
    less. Lineage and `reads` use the same rule (P4.11). Anything but
    a read, select, function or recursion is refused, save the
    `query_only` pragma the session itself runs. `SqlQuery::raw` switches
    to recording only: the person's explorer, through IPC `query_sql
    { raw: true }` — the MCP tool has no such switch — and `save_lens`
    checks its query through the enforced path, so a raw query can't
    become a lens. A model's compile reads its sources in record mode. Installed before `prepare` in a `ReadSession`
    that, when dropped — on every path, a panic included — clears it and
    `PRAGMA query_only`, so the pooled connection comes back writable. The
    result's `reads` is `{ models, tables }`: every view read, directly or
    through another view (a `count(*)` over a view reports only its base
    table, with the view as accessor, so view accessors count too), and
    every stored table the query's own SQL read — not one a view read, and
    not a table-valued function such as `json_each`. A CTE's name can be an
    accessor, so a CTE body counts as the query's own SQL.
  - **Parameters bind by the statement's own list:** a positional count
    must match; every `:name` the statement uses must be given (a name it
    doesn't use is ignored, so a host passes one fixed set).
  - Executes under `PRAGMA query_only = ON`. A test proves the pooled
    connection stays writable after a failure at each stage.
  - A 5 s interrupt timer (`InterruptHandle`) stops runaway queries.
  - The row cap defaults to 500 and can be raised to at most 10 000.
    `truncated: true` means more rows existed.
  - Caller mistakes (bad SQL, writes, timeouts) are `Invalid` errors: the
    IPC `INVALID` code, and MCP `invalid_params`.
  - `reads.tables` is empty except in a raw read. Every bundled lens,
    advisory, source input and built-in entity metric reads only views —
    audit tests (`bundled_queries_read_only_published_views`,
    `builtin_entity_metrics_read_only_views`) keep it so. A missing column
    is a reason to extend a model, not to reach around it.
- Values are `SqlCell`: an untagged `null | boolean | number | string`,
  so the TS binding is a plain scalar union. Blobs come back as
  `"<blob N bytes>"`.
- The catalog is SQL: `v_model` (name, owner, kind, description) and
  `v_model_column` (each column's SQL type and doc). The MCP
  `describe_schema` tool and its IPC twin are gone (tsk517).
- Lenses never call models or run collectors on render; they read what
  collectors already produced.
- Everything an extension adds (entities, relations, dimensions, metrics)
  appears in the same models (`v_model`, `v_dimension`, `v_metric_spec`) and
  the same `query_sql` as core data. Agents never need
  an extension-specific tool. Full agent surface:
  [extensions.md](./extensions.md) → "Agents: the MCP surface".

## Collectors (P7.B3)

A **collector** brings data in: a program (`exec`), a sandboxed script
(`starlark` / `jaq`) or a provider's read (`read`). It writes
**entities** (rows a model can `ref()`) or **facts** (measurements on
declared measures), never both. One declaration, `oxplow_config::collectors`
(`CollectorSpec`, `parse_collectors`), serves `extension.yaml` (v1
`sources:` is migrated to it, each `schedule:` becoming a `trigger:`) and
`.oxplow/project.yaml`. The real, tested example
is `examples/extensions/github/` (PRs from the GitHub API via `gh` or
`GITHUB_TOKEN`); the user guide is `docs/guide/lenses.md`.

```yaml
collectors:
  - id: prs                    # dotted ids are fine: repo.scan_clone
    doc: The repo's recent pull requests.
    runtime: exec              # or starlark / jaq (derived, below) / read
    entry: sync.sh             # relative, inside the extension folder
    trigger: { every: 15m }    # or manual; { on: [<event type>], where?: {field: value} }
    env: [GITHUB_REPOSITORY]   # host env vars passed through; nothing else is
    credentials: [GITHUB_TOKEN] # keychain secrets, set in Settings → Extensions
    entities:
      - name: pr               # view: v_<extension>_<entity> = v_github_pr
        doc: One pull request.
        key: number
        columns:               # ordered; `type` or `{type, doc}`
          number: int          # text | int | real | bool | time
          title: { type: text, doc: PR title }
        relations:             # documented joins (not executed)
          - { to: v_task, on: "v_github_pr.title LIKE '%tsk' || v_task.id || '%'" }
```

The entry prints `{"entities": {"<name>": [ {col: value, …}, … ]}}`.

**Entity vs fact collectors.**

- An **entity collector** (`entities:`) writes rows published as
  `v_<owner>_<entity>`; `collector_runner` runs it. It may be `exec`
  (approved), `starlark` / `jaq` (derived, below; may call `ai_*`, has no
  `files()`) or `read`.
- A **fact collector** (`facts: [<measure>, …]`) is what a *gauge* was
  until P7.B3. It runs in the fact engine (`MetricsService`,
  [metrics.md](./metrics.md)): its Starlark gets the snapshot tree
  (`files()` / `source_files()` / `ast_query()`, the `TreeHost`) but no
  `ai_*`; its input is `{report?, rows?, event?}` (`report: { path,
  format }` parsed — text | json | xml | lcov | lines; the `input:`
  query's rows; the trigger event); it returns `{"facts": [{measure,
  value, subject?, path?, line?, rule?, num?, den?, dims?}]}` and nothing
  else (`facts_of`). A fact on a measure not in `facts:` or not defined is
  dropped. Each run records one metric capture (or a failed one) plus a
  `collector_run` row and `collector.synced@1` with its `facts` count.
  Snapshot runs keep the delta / full-baseline machinery (pending-baseline
  queue, `scan_kind`, fingerprint, dominated-capture prune).

**Owners.** An extension (its name), `project` (`.oxplow/project.yaml`'s
`collectors:`) or `built-in` (the bundled `oxplow.*` code metrics, run
only when `metrics: - use: oxplow.<x>` enables them). `project` and
`built-in` are reserved extension names. There are no global collectors.

- **The project's collectors record facts** (`collectors:` in
  `.oxplow/project.yaml`; scripts under `oxplow/collectors/`); an entity
  collector belongs in an extension.
- **An extension's fact collector is sandboxed** (starlark or jaq). Only
  the project's own fact collectors may be `exec`, and those need a
  person's approval (Settings → Data → Programs; `exec_consent`
  `ProgramKind::Collector`, key `collector:<id>`).
- **A fact collector gets no `env` / `credentials` / `network`**, and
  `report` is for fact collectors only.

**Migrating `gauges:`.** A `gauges:` block in either file is a load
error naming the fix: `oxplow plugin migrate --project` (project.yaml) or
`oxplow plugin migrate <name>` (an extension; it also does the v1→v2
manifest migration). Both rewrite the block in place, textually and
idempotently: `key`→`id`, `title`→`doc`, `compute.runtime`→`runtime`,
`compute.entryFile`→`entry`, `emits`→`facts`, `compute.report` +
`compute.input` → `report: { path, format }`, trigger `on-snapshot` (the
default) → `{ on: [snapshot.taken] }`, `on-effort-complete` → `{ on:
[effort.finished] }`, `manual` → `manual`. `on-report` / `continuous` and
`compute.args` have no equivalent — the migration refuses, naming the
gauge. Fingerprints are preserved, so a migrated gauge keeps its
baseline.

**Running.** `on:` collectors, entity and fact alike, run from the
`collector.triggers` consumer (below). For a fact collector:
`snapshot.taken` runs it only when the take recorded files (not
`unchanged`, `file_count > 0`); `effort.finished` runs it over the
effort's end snapshot; any other type over the stream's latest snapshot.
`every:` collectors run from the scheduler as the system through
`collector.sync`. **`collector.sync { owner, id }` is the one manual run
for every collector** — it replaced `source.sync` and `metric.run`; for a
fact collector it returns `facts`. (`metric.rebuild` still runs the
whole-tree baseline.)

**Parse rules.** `entry` for exec/starlark/jaq; `provider: { instance,
collector }` (and nothing a script needs) for `read`, whose records land
in the capability's model and which `provider.sync` runs; `env`,
`credentials`, `network` only on an exec entity collector; `input` only on
starlark/jaq; exactly one of `entities` / `facts`; `report` only with
`facts`; a project collector must have `facts`; an extension's fact
collector isn't `exec`; `after` and `where` only with `on:`,
whose types must be registered (`v_event_type`); `oxplow.` ids are
oxplow's own.

**An entity collector's run** (`collector_runner::run_collector`) commits its rows, its
`collector_run` row and its `collector.synced@1` event in **one
transaction**; a failed run commits the failure the same way (no rows,
the last good counts and checkpoint kept). The event's subject is
`collector:<owner>/<id>` (ref kind `collector`), its `trigger` is
`manual` (`collector.sync` by an actor), `every` (the scheduler, as the
system) or `on`. `collector_run` (V133, `v_collector_run`; it replaced
V75 `ext_source_state`) keeps `owner, id, status (ok | error |
needs_approval), last_run_at, error, row_counts_json, cursor_json,
last_event_id`. The UI refreshes when `v_collector_run` changes
(`collectorRan`); the in-memory `SourceSynced` is gone.

**Health (P7.C2).** Collectors share the plugin failure policy
([extensions.md](./extensions.md) "Health, disable and repair"): three
failed runs in a row disable one (`plugin_health`, key owner / id, kind
`collector`); then nothing runs it until a person's `plugin.enable`, and
a lens over its view warns that its rows aren't refreshing.

**`on:` triggers** (`collector_triggers.rs`, the `collector.triggers`
async pump consumer). When an event an enabled collector's `on:` names is
logged — and each `where` field of its payload equals its value — the
consumer runs that collector for it (`collector_runner::run_for_event`),
serially, as the system. `where` values are YAML text compared by kind
(`payload_matches`): a string exactly, a boolean as `true`/`false`, a
number by value (`file_count: 1` matches `1.0`); **a field the payload
lacks never matches**, so `where: { trigger: git_refs }` skips every event
type without a `trigger` field:

- **Input.** The `input` SQL binds the event's anchors by name
  (`:stream_id`, `:snapshot_id`, `:effort_id`, `:thread_id`, `:turn_id`,
  integers, and `:event_id`, its seq; NULL when absent), and the script
  gets the event as `input.event` (`type`, `seq`, `id`, `at`, `subject`,
  `payload`, `anchors`).
- **Once per event.** The run's `collector_run.last_event_id` is the
  event's seq, and its `collector.synced@1` is caused by the event with a
  per-event dedupe key. They commit with what the run wrote — an entity
  collector's rows, or a fact collector's capture and facts
  (`RunLog::record_with`, tsk712) — so a redelivered event writes nothing,
  and no run lands without its record.
- **Failure.** A failing collector is recorded and announced like any run
  and doesn't dead-letter the event: one broken collector never holds up
  the rest. An exec collector nobody approved records `needs_approval`
  (no event) and runs nothing.
- **Order.** An event waits on the `after:` lists of the collectors **it
  triggers** (`after_for(event_type)`), limited to consumers the pump has
  (an unknown name is logged and ignored): an `effort.finished` event waits
  for `change.analyze` because `effort_churn` names it, while a
  `snapshot.taken` event doesn't (tsk711). `on: [collector.synced]` is
  refused at parse.

**Derived collectors** (`runtime: starlark` or `jaq`, tsk323) compute
entities from data already in the semantic layer:

```yaml
  - id: hot
    runtime: starlark          # def transform(input): return {"entities": {...}}
    entry: sources/hot.star
    input: "SELECT id, title FROM v_task WHERE priority = 'high'"
    entities: [...]
```

- **Input.** `input` is read-only SQL through the SQL gateway.
  More than 10k rows fails the run rather than deriving from a partial
  set. The script gets `{"rows": [{col: value, …}]}` and returns the exec
  shape.
- **Sandbox.** It runs in the collector sandbox
  (`run_sandboxed_excluding` + `run_starlark_with_ai` / `run_jaq`), with
  no files, network, env or secrets. So there is **no approval**:
  `list_collectors` reports it as approved, and the scheduler runs it on
  its `every:` trigger. Its one way out is oxplow's own: the `ai_*` builtins
  (`ai_classify` / `ai_score` / `ai_summarize` / `ai_extract`), recorded
  computations on the project's AI roles as caller `collector:<owner>/<id>`,
  whose wait is left out of the budget ([ai-providers.md](./ai-providers.md)
  "`ai_*` functions for sources").
- **Refused at parse:** `env` / `credentials` on a derived collector,
  `input` on an exec collector, and an `input` that names one of the
  collector's own views (a collector can't feed on itself). Reading other extensions' views
  is fine.
- **Scripts** are read with `extensions::read_extension_file`, so bundled
  extensions can ship them.

**Incremental sync** (`sync: upsert`, tsk323; default `replace`):

- **What a run writes.** The output may add
  `"deleted": {"<name>": [key, …]}`. Each mentioned entity's rows are
  inserted or replaced by key and its tombstoned keys deleted, in the one
  transaction (`collector_store::write_rows_tx` with `EntityWrite::Upsert`).
- **Unmentioned entities** are left alone. A `replace` collector empties
  them instead.
- **Refused:** `deleted` from a `replace` collector.
- **Schema changes.** A changed column set still rebuilds the table, so
  the first run after one holds only that run's rows.
- **Row counts** are the entity's totals after the write.

**Code map.**

| Piece | Where |
|---|---|
| Parse/validate declarations (`CollectorSpec`, `Trigger`) | `crates/oxplow-config/src/collectors.rs` |
| `gauges:` → `collectors:` (`migrate_gauges_text`, behind `oxplow plugin migrate --project` / `<name>`) | `crates/oxplow-config/src/collectors.rs`, `apps/desktop/src-tauri/src/plugin_cli.rs` |
| Fact collectors: `FactCollector`, `fact_collectors()`, `run_collector_by_key`, `run_snapshot_collectors` / `run_effort_collectors` / `run_event_collectors` | `crates/oxplow-app/src/metrics_service.rs` |
| Fact-collector script host (`TreeHost`, `run_fact_starlark`, `facts_of`, `parse_report`) | `crates/oxplow-collect-plugin/src/lib.rs`, `runtime.rs` |
| v1 `sources:` / `schedule:` → `collectors:` / `trigger:` | `crates/oxplow-app/src/extensions/migrate_v1.rs` |
| Consent (`approve_reviewed`), exec, coercion, `run_collector` / `run_for_event`, `CollectorRunner` + the `collector.sync` command, scheduler | `crates/oxplow-app/src/collector_runner.rs` |
| The `collector.triggers` consumer (`on:` / `where` / `after`) | `crates/oxplow-app/src/collector_triggers.rs` |
| Entity tables + views, run state (V133 `collector_run`) | `crates/oxplow-db/src/collector_store.rs` |
| Settings → Data read model (`data_entities`) | `crates/oxplow-app/src/semantic_catalog.rs` |
| IPC `list_collectors` / `approve_collector` / `set_credential` (UI only); running is `run_command collector.sync` | `crates/oxplow-rpc/src/commands/collectors.rs` |
| MCP `list_collectors` / `run_collector` / `preview_collector` (the `collector.sync` command as the agent; never approves) | `crates/oxplow-mcp/src/lib.rs` |
| UI: Settings → Data (entities + counts, collector rows, Run) | `apps/desktop/src/components/DataSection.tsx` |
| UI: credentials per extension | `apps/desktop/src/components/ExtensionsSection.tsx` |
| IPC `list_data_entities` (models + counts, unsynced entities) | `crates/oxplow-rpc/src/commands/semantic.rs` |

**Decisions (epic tsk289, 2026-09-27).**

- **Storage is a re-syncable cache in the main DB,** not the attached
  per-extension DB the first design sketched. Each entity is a table
  `ext__<extension>__<entity>` (dashes → underscores) plus its
  `v_<extension>_<entity>` view.
  - Oxplow does all the writing from the declared schema, and no
    extension SQL runs on writes, so isolation buys little.
  - One pool and plain views keep `query_sql` simple.
  - Every run writes all of a collector's entities in **one transaction**,
    so a failed run changes nothing.
  - A changed column set rebuilds the table.
  - The store refuses to replace a view it doesn't own: a core view, or
    one from another extension. Extension `task` plus entity `note` can't
    shadow `v_task_note`.
  - `drop_extension` removes an extension's tables, views and state.
- **Consent.** An exec collector runs code, so it runs only after a
  person approves it.
  - Approval is bound to the entry script's SHA-256 and stored per
    machine outside the repo, MACed under a keychain key (see
    [architecture.md](./architecture.md) → "A repo's config never runs a
    program without consent").
  - A teammate who pulls the repo approves it themselves, and a changed
    script needs re-approval.
  - Approving is its own UI-only step (`approve_collector`, the version
    the person reviewed); running is the `collector.sync { owner, id }`
    command, which never approves — from the UI, a lens action, the
    scheduler or MCP `run_collector` alike — so an agent can't consent on
    a person's behalf. The approval key stays `<owner>/<id>`.
- **Environment.** The entry gets `PATH`, `HOME`, its declared `env`
  names, its declared `credentials`, `OXPLOW_EXTENSION_DIR` and
  `OXPLOW_COLLECTOR_ID`. It runs with a
  120 s timeout and a 64 MB stdout cap, and both pipes are drained so it
  can't deadlock.
- **Network** (tsk324, `net_sandbox.rs`). `network: [api.github.com,
  "*.githubusercontent.com"]` lists the hosts an exec collector may reach:
  - **Pattern syntax.** Lowercase names; `*.` means subdomains only, not
    the bare domain.
  - **Part of the approval.** `approval_hash` is the entry hash plus the
    sorted host list, so widening the list needs re-approving. With no
    `network`, it is just the entry hash, so older approvals stay valid.
  - **macOS enforcement** (`net_sandbox::enforced()`: `/usr/bin/sandbox-exec`
    exists). The entry runs as
    `sandbox-exec -p PROFILE <entry>`, where PROFILE is `(allow default)`
    minus outbound network, except unix sockets (the resolver) and
    `localhost:*`.
  - **The proxy.** A per-run `EgressProxy` (tokio, `127.0.0.1:<random>`)
    handles `CONNECT host:port` and absolute-form `http://` requests. It
    forwards only declared hosts, rewrites the request line to origin
    form, and answers anything else `403`. The entry gets
    `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`, in both cases, pointing at
    it. Direct connections fail in the sandbox, so the proxy is the only
    way out. No `network` means no egress.
  - **Gaps.** Other localhost ports stay reachable. Tools that ignore
    proxy variables simply can't connect. Other OSes run unsandboxed, and
    the listing's `network_enforced` is false, so the approval text says
    "not enforced on this OS". Linux would need unshare/seccomp or a
    network namespace.
  - **Verified** (2026-09-27, macOS 27): `gh api` and `curl` in the GitHub
    example sync through the proxy. The macOS-gated test
    `a_sandboxed_source_reaches_only_its_declared_hosts` pins
    declared → 200, undeclared → 403, and direct → blocked.
  - **Derived collectors** can't declare `network`.
- **Credentials.** `credentials: [NAME]` declares secrets the entry gets
  as env vars. Values live in the OS keychain (`Services.secrets`, shared
  with AI provider keys) under account
  `source:<project key>:<extension>:<NAME>` (tsk348; the project key is a
  hash of the canonical project path, shared by its worktrees). So neither
  another extension nor a same-named extension in another repo can read
  one by declaring the same name. Credentials set before tsk348 (no
  project in the account) aren't read; set them again.
  - Only a person sets them: IPC `set_credential` (UI-only in the parity
    manifest; only names some collector or provider of that extension
    declares).
    Listings carry `credentials: [{name, set}]`, never values.
  - An unset credential is simply not passed; the script decides (the
    GitHub example falls back to `gh`). A keychain error fails the run.
  - A name can't be in both `env` and `credentials`, or be `PATH`, `HOME`
    or `OXPLOW_*`.
  - Changing `credentials` doesn't need re-approval: approval is bound to
    the entry script, and a secret reaches it only after the person sets
    that extension's value.
  - `collector_runner::Collectors` bundles root, approvals, store, the
    database and event schemas, secrets and the gateway — the context
    every list/run/set call takes (`Collectors::of(svc, root)`).
- **Where it runs.** Collectors run from the **primary** stream's worktree,
  and their data is project-global, like dashboards.
- **Errors.**
  - `RunCollectorError` distinguishes `NotFound`, `NeedsApproval`
    (nothing ran, nothing recorded), `Failed` (it ran; recorded with its
    event, keeping the last good row counts) and `Storage`.
  - A lens reading an uncollected entity gets "reads `v_x`, which hasn't
    been collected yet. Run collector `ext/id`…" in place of SQLite's
    bare "no such table".
- **Scheduling.** A background loop (`spawn_scheduler`, started from boot)
  calls `run_due_collectors` once a minute: every approved collector with
  an `every:` trigger that's due (`due_collectors`, which is pure and
  tested) runs as the `collector.sync` command with `Actor::System` — so
  a scheduled run is audited and logs `command.executed` like one from
  the UI (tested: `the_scheduler_runs_collector_sync_through_the_bus`).
  Unapproved collectors never run unattended: the command refuses them.
- **Schema.** A synced entity is a model its extension owns: its
  description is its doc plus the joins it documents (`relations`), and
  its contract is its declared columns with their docs, refreshed when the
  declaration changes (`register_entity`). Settings → Data also lists a
  declared entity that hasn't synced (`kind: declared`, no count).

## Change analysis

A **change** is one diff: a `commit` (vs its first parent), an `effort`
(start snapshot → end snapshot, or → the working tree while open), an
agent `turn` (its `start_snapshot_id` → its end `snapshot_id` — what the
turn changed, even across a snapshot taken mid-turn; a turn still
running is `NotFound`; target `{"kind":"turn","turnId":"trn12"}`, P2.10,
V101) or a stream's `working` tree (vs HEAD). V101 rebuilt `change` to
widen its kind CHECK, emptying the cache first (rows are recomputed on
demand). `crates/oxplow-app/src/change_analysis.rs`
analyzes it and stores the rows behind `v_change*`; lenses (the
oxplow-analytics change cards) only read them.

- **Getting one.** `ensure_change(target)` (IPC and MCP; not hinted read-only, since it stores the analysis, tsk371) returns
  the `v_change` row, computing first if needed. It diffs its two
  revisions (`v_change.base_revision` → `head_revision`) through `Trees`
  (shared with the `diff` IPC, `.context/vcs.md`), reads the
  first 200 changed files' contents, runs `code_analysis::analyze_files`
  (tree-sitter metrics per side, churn, import deltas), and builds rows:
  - files: status, +/−, zone (project zone rules), `is_test`. The "look
    here first" score left core (P7.B5, V137 dropped its columns,
    `v_change_file` v2): oxplow-analytics' model `change_interest`
    (`v_oxplow_analytics_change_interest { change_id, path, interest,
    reasons }`) computes it in SQL over `change_file` and
    `change_function` — `(1 + log2(1+lines)) × (1 + 0.6·Σcomplexity↑) ×
    (1 + 0.4·Σparams↑) × (1 + (longest new fn − 60)/40)` — using SQLite's
    math functions (`LIBSQLITE3_FLAGS = -DSQLITE_ENABLE_MATH_FUNCTIONS`
    in `.cargo/config.toml`);
  - functions: added / deleted / modified (signature and/or body), deltas,
    churn and churn share; unchanged ones aren't stored;
  - imports: added/removed with zones, `cross_zone` for new boundary
    crossings;
  - co-change left core (P7.B5, V138 dropped `change_co_change`):
    oxplow-analytics' `co_change_pair` (materialized over the commit
    index: pairs sharing ≥ 3 commits of ≤ 50 files in 180 days) and
    `change_co_change` (a change's files dormant 90+ days, or whose top
    three co-changers are all absent) compute it in SQL;
  - test files (`v_change_test_file`, V83): for each changed file that is
    a test file or has tests on either side (Rust's inline `mod tests`
    counts), test functions plus assertion calls and skip markers
    before and after (`test_signals::count`, a heuristic across
    languages; `//` and `#` comment lines don't count, so commenting an
    assertion out lowers the count).
- **Duplicates** come later: a background whole-tree scan scoped to the
  changed files (`duplication_scan::DuplicationRecorder`) stores
  `v_change_duplicate` — only while the analysis it belongs to is still
  the change's latest (`change.events_to` unchanged). The same scan is
  recorded as a code-quality scan (`v_code_quality_scan` /
  `v_code_quality_finding`). It writes **no** `oxplow.duplicate_lines`
  facts: only a full-tree scan may restate that metric (tsk365) — the
  built-in collector's, on every ref move (P7.B5,
  [code-quality.md](./code-quality.md)).
  Closed efforts and turns scan their end snapshot: `Trees` reads any
  revision (P5.B2).
- **Caching.** A change is keyed by (stream, kind, target) — commit shas are
  resolved to full ids, so `HEAD` and a short sha share one row. Commits,
  turns and closed efforts are computed once (an effort that closes since
  is recomputed: its head moved). `ensure_change` reads a working tree's
  or an open effort's stored analysis, computing it the first time.
- **Keeping it current (P7.B4).** The `change.analyze` async pump consumer
  (`change_reactor.rs`) recomputes them as the stream moves: on a
  `snapshot.taken` that recorded files (not `unchanged`) or a
  `vcs.head.moved`, `refresh_change` re-analyzes the stream's `working`
  change and every open effort's (one without a start snapshot is
  skipped). On `effort.finished` it recomputes that effort's change
  against its end snapshot: the effort closes before its end take, so no
  take event reaches it while open, and an effort that edited and
  finished within one turn would otherwise keep a stale (or no) analysis
  — the one `effort_churn` (`after: [change.analyze]`) reads (tsk710). A
  burst analyzes once: an event a newer qualifying one for the same
  stream supersedes is skipped. A change already being computed
  defers the event (`Busy`, retried); a failure is a dead letter naming
  the stream. Each analysis stamps `change.snapshot_id` (what it was
  computed against) and `events_to` (the log's highest seq as it began;
  `v_change` v3). Results are announced by their commit (`ModelsChanged`
  on `v_change*`): `useChange` re-ensures when `v_change` changes. The
  in-memory `ChangeStale` / `ChangeAnalyzed` / `CodeQualityScanned` and the
  invalidation loop are gone. Concurrent requests for the same change
  return it `running`.
- An effort without a start snapshot is an error, not an empty diff.

Deleted / skipped tests and removed assertions are `v_change_function`
(deleted `is_test` rows) plus `v_change_test_file`, read by the
oxplow-review Tests Weakened lens. Missing co-change is oxplow-analytics'
`v_oxplow_analytics_change_co_change`.

**Still target:** network enforcement off macOS; AI-role columns.
(Extension-declared metrics and dimensions, fact and entity, are current:
see extensions.md.)

## Relation to other docs

- [metrics.md](./metrics.md): the fact substrate this generalizes. Still
  authoritative for facts, captures, the cube and fact collectors.
- [extensions.md](./extensions.md): how lenses and extensions consume this
  layer.
- [ai-providers.md](./ai-providers.md): the AI functions sources and lenses
  can use.
- [collection.md](./collection.md): test-run and coverage ingest, which
  becomes the tests & coverage source.
