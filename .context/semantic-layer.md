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
>   metrics and gauges (tsk311, see extensions.md).
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

| Source | Entities | Facts |
|---|---|---|
| work | stream, thread, task, effort, decision, claim | cycle time, steering, lifecycle |
| knowledge | wiki_page, comment, correction | freshness |
| git | commit, branch, file_change, cochange, blame_line | churn |
| snapshots | snapshot, change_file, test_change | — |
| lsp | diagnostic, symbol, reference | diagnostic counts by severity |
| tests & coverage | test_run, test_case | coverage, pass/fail |
| code metrics | function, file | complexity, length, params |
| agent | session, turn, tool_call, agent event (`v_event` `agent.*`), context_read | tokens, struggle |
| ai | ai_call | tokens, latency |
| usage | page_visit | — |

New primitives that don't exist yet:

- `decision`: the forks an agent resolved during an effort (fork, choice,
  alternatives, confidence, why). Written via a `record_decision` MCP tool.
- `claim`: statements like "tests pass", joined to observations so each one
  reads as verified or unverified. Written via `record_claim`. Both attach to the given task's open effort, or else the
  thread's; a given task must be in the calling thread's stream (tsk353).
- `context_read`: which `.context/*.md` docs the agent read before touching
  a subsystem.
- `struggle`: retries, repeated reads of the same file, reverted edits.

These exist because what a human reviewing agent work needs most is
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
- checks **lineage**: what the authorizer reports the view reading
  directly (`ReadSession::direct_inputs`) must be exactly its declared
  `ref()`s and `source()`s — reading a table without `source()` fails;
- checks the **contract**: the view's `PRAGMA table_info` must equal the
  declared columns, and `model_contract` holds what each `(view, version)`
  promised — a changed contract at the same version fails naming the
  column ("bump its version");
- records `model` (view, name, owner, version, description, compiled
  SQL), `model_input` (`ref` | `source`) — V105.

**Who owns a view** is the registry's answer (P4.9, V109): `model.kind`
is `sql` (compiled from a model file at every open — `drop_all` drops
only these) or `entity` (an extension's synced entity, created by
`ext_source_store::write_entity` when its source runs and kept across
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
- A `metric_grid()` query's `reads.measures` pairs with
  `MetricSamplesChanged { measures }`: facts land on every OTLP burst, and
  the measure scope keeps a metric tile quiet unless its own measures
  moved (tsk198).
- **The UI** (`src/lens/lensRerun.ts`): every lens host — the lens page,
  slots, dashboard lens tiles, the rail's alerts and the explorer — keeps
  its last run's `reads` and re-runs through `useRerunOnChange` when
  `modelsChanged` names a model it read, `metricSamplesChanged` names one
  of its measures (an empty list is "unknown", so it re-runs), or a lens
  definition under `oxplow/extensions/` changed. Nothing else re-runs it:
  the old everything-but-two deny-list (`shouldRerunLens`) and its 750 ms
  debounce are gone; a burst of commits coalesces into one re-run
  (100 ms).

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
| `v_decision` | forks the agent resolved (question, choice, alternatives, confidence, why). `provenance`: `recorded` via MCP `record_decision` (V76), or `inferred` by the summarize model when the effort closed (V79) |
| `v_claim` | agent claims ("tests pass") with `verified` (cited evidence, or a `tests_pass` claim whose effort's **latest** `v_test_run` — its own or one claimed through attribution — has `failed = 0 AND total > 0`) (V76; latest-run rule V91, tsk366) |
| `v_tool_call` | every agent tool call: the `tool_call.project` pump consumer's projection of `agent.tool.finished` (one row per event; the ingest parses the payload with `oxplow-app/src/tool_calls.rs`; paths worktree-relative; Bash `ok` is NULL when Claude reports no exit code) (V77; `turn_id`, `event_id` since V102) |
| `v_context_read` | `Read`s of `.context/*.md` (V77) |
| `v_struggle` | per effort: a file edited 5+ times, or 3+ failed commands (V77) |
| `v_ai_call` | oxplow's own model calls: role, provider, model, caller, tokens, latency, ok/error (V78; see [ai-providers.md](./ai-providers.md)) |
| `v_metric_spec` | metric definitions: aggregation, direction, target / warn / fail (enabled lives in project.yaml) (V80) |
| `v_agent_nudge` | guidance oxplow sent the agent mid-effort (V80; `turn_id`, `delivered_at` since V102) |
| `v_code_quality_scan`, `v_code_quality_finding` | code-quality scans and their findings (duplicate blocks, with the peer in `extra_json`) (V80) |
| `v_dashboard`, `v_dashboard_item` | user dashboards and their tiles (V80) |
| `v_effort_metric_delta` | per effort, how each metric moved (baseline → current, `crossing`). **Stored, not a live query:** the metric engine computes it (`CollectionService::refresh_effort_evidence`) and `effort_evidence.rs` refreshes it on `effort.finished` (the `effort.evidence` pump consumer) and, debounced 3 s, for open efforts on metric/observation/token events, then emits `EffortEvidenceChanged` (V80) |
| `v_effort_observation` | per effort, test runs / diff coverage / analysis rebuilt from its claimed captures; refreshed the same way (V80) |
| `v_change`, `v_change_file`, `v_change_function`, `v_change_import`, `v_change_co_change`, `v_change_duplicate` | stored change analysis (see "Change analysis" below) (V81) |
| `v_commit`, `v_commit_file`, `v_commit_task`, `v_branch` | git history and branches (V85). The commit indexer (`commit_indexer.rs`, run at boot and on `GitRefsChanged`) stores the last 500 commits reachable from the **primary** worktree's HEAD with their files (first-parent diff) as it projects them into `page_ref`; `v_commit_task` reads the indexer's task-mention edges. `refresh_branches` restates `v_branch` from `list_branches` (heads included) and maps each local branch to the stream checked out on it. The old `task_commit` table is unused |
| `v_test_run`, `v_test_case` | test runs (V87), views only. A run IS its `metric_capture` (producer `tests`, or `test-run` for a run that measured nothing), and both views read its verbatim `test-detail` payload in `detail_json`: counts from the payload, cases by `json_each` over `suites[].cases[]`. Cases come from the payload, not the `oxplow.test_case` facts, because those are skipped while every tests metric is disabled. `effort_id` is the effort whose ledger claims `run:<id>` (kind `run`), else the capture's own. A run that only reported counts (MCP `record_test_run`) has no cases |
| `v_diagnostic` | what the language servers have published, right now (V86). `lsp_diagnostics.rs` subscribes to the LSP session broadcast and replaces a file's rows per `(stream, language, path)` on each `textDocument/publishDiagnostics` (paths repo-relative, positions 1-based, a URI outside the worktree dropped). Live state: the table is cleared at boot and a server's rows when it restarts, crashes or stops. Only files a server has published appear (usually the open ones, not the whole repo). A `DiagnosticsChanged` event goes out at most every 500 ms per stream, from the first change, so a server that publishes continuously can't starve it |

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
    its base table at top level after the view's own reads). Anything but
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
- Lenses never call models or run sources on render; they read what sources
  already produced.
- Everything an extension adds (entities, relations, dimensions, metrics)
  appears in the same models (`v_model`, `v_dimension`, `v_metric_spec`) and
  the same `query_sql` as core data. Agents never need
  an extension-specific tool. Full agent surface:
  [extensions.md](./extensions.md) → "Agents: the MCP surface".

## User and extension sources (current)

Extensions declare **sources**: code that pulls external records into the
semantic layer as entities. The real, tested example is
`examples/extensions/github/` (PRs from the GitHub API via `gh` or
`GITHUB_TOKEN`); the user guide is `docs/guide/lenses.md`.

```yaml
sources:
  - id: prs
    doc: The repo's recent pull requests.
    runtime: exec              # or starlark / jaq (derived, below)
    entry: sync.sh             # relative, inside the extension folder
    schedule: every 15m        # or manual; every <n>m | <n>h
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

**Derived sources** (`runtime: starlark` or `jaq`, tsk323) compute
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
- **Sandbox.** It runs in the collector sandbox (`run_sandboxed` +
  `run_starlark` / `run_jaq`), with no files, network, env or secrets.
  So there is **no approval**: `list_sources` reports it as approved, and
  the scheduler runs it on its `schedule`.
- **Refused at parse:** `env` / `credentials` on a derived source, `input`
  on an exec source, and an `input` that names one of the source's own
  views (a source can't feed on itself). Reading other extensions' views
  is fine.
- **Scripts** are read with `extensions::read_extension_file`, so bundled
  extensions can ship them.

**Incremental sync** (`sync: upsert`, tsk323; default `replace`):

- **What a run writes.** The output may add
  `"deleted": {"<name>": [key, …]}`. Each mentioned entity's rows are
  inserted or replaced by key and its tombstoned keys deleted, in the one
  transaction (`ext_source_store::write_rows` with `EntityWrite::Upsert`).
- **Unmentioned entities** are left alone. A `replace` source empties
  them instead.
- **Refused:** `deleted` from a `replace` source.
- **Schema changes.** A changed column set still rebuilds the table, so
  the first run after one holds only that run's rows.
- **Row counts** are the entity's totals after the write.

**Code map.**

| Piece | Where |
|---|---|
| Parse/validate declarations | `crates/oxplow-app/src/extension_sources.rs` |
| Consent, exec, coercion, `run_source`, scheduler | `crates/oxplow-app/src/source_runner.rs` |
| Entity tables + views, run state (V75 `ext_source_state`) | `crates/oxplow-db/src/ext_source_store.rs` |
| Settings → Data read model (`data_entities`) | `crates/oxplow-app/src/semantic_catalog.rs` |
| IPC `list_sources` / `run_source(approve?)` | `crates/oxplow-rpc/src/commands/sources.rs` |
| MCP `list_sources` / `run_source` (never approves) | `crates/oxplow-mcp/src/lib.rs` |
| UI: Settings → Data (entities + counts, source rows, Run) | `apps/desktop/src/components/DataSection.tsx` |
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
  - Every run replaces all of a source's entities in **one transaction**,
    so a failed run changes nothing.
  - A changed column set rebuilds the table.
  - The store refuses to replace a view it doesn't own: a core view, or
    one from another extension. Extension `task` plus entity `note` can't
    shadow `v_task_note`.
  - `drop_extension` removes an extension's tables, views and state.
- **Consent.** A source runs code, so it runs only after a person
  approves it.
  - Approval is bound to the entry script's SHA-256 and stored per
    machine outside the repo, MACed under a keychain key (see
    [architecture.md](./architecture.md) → "A repo's config never runs a
    program without consent").
  - A teammate who pulls the repo approves it themselves, and a changed
    script needs re-approval.
  - The IPC `approve` flag exists only on the UI path. MCP `run_source`
    never approves, so an agent can't consent on a person's behalf.
- **Environment.** The entry gets `PATH`, `HOME`, its declared `env`
  names, its declared `credentials`, `OXPLOW_EXTENSION_DIR` and
  `OXPLOW_SOURCE_ID`. It runs with a
  120 s timeout and a 64 MB stdout cap, and both pipes are drained so it
  can't deadlock.
- **Network** (tsk324, `net_sandbox.rs`). `network: [api.github.com,
  "*.githubusercontent.com"]` lists the hosts an exec source may reach:
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
  - **Derived sources** can't declare `network`.
- **Credentials.** `credentials: [NAME]` declares secrets the entry gets
  as env vars. Values live in the OS keychain (`Services.secrets`, shared
  with AI provider keys) under account
  `source:<project key>:<extension>:<NAME>` (tsk348; the project key is a
  hash of the canonical project path, shared by its worktrees). So neither
  another extension nor a same-named extension in another repo can read
  one by declaring the same name. Credentials set before tsk348 (no
  project in the account) aren't read; set them again.
  - Only a person sets them: IPC `set_source_credential` (UI-only in the
    parity manifest; only names some source of that extension declares).
    Listings carry `credentials: [{name, set}]`, never values.
  - An unset credential is simply not passed; the script decides (the
    GitHub example falls back to `gh`). A keychain error fails the run.
  - A name can't be in both `env` and `credentials`, or be `PATH`, `HOME`
    or `OXPLOW_*`.
  - Changing `credentials` doesn't need re-approval: approval is bound to
    the entry script, and a secret reaches it only after the person sets
    that extension's value.
  - `source_runner::Sources` bundles root, state dir, store and secrets,
    the context every list/run/set call takes (`Sources::of(svc, root)`).
- **Where it runs.** Sources run from the **primary** stream's worktree,
  and their data is project-global, like dashboards.
- **Errors.**
  - `RunSourceError` distinguishes `NotFound`, `NeedsApproval` (nothing
    ran), `Failed` (it ran; recorded as the source's state, keeping the
    last good row counts) and `Storage`.
  - `SourceSynced` is emitted only when the source actually ran.
  - A lens reading an unsynced entity gets "reads `v_x`, which hasn't
    synced yet. Run source `ext/id`…" in place of SQLite's bare "no such
    table".
- **Scheduling.** A background loop (`spawn_scheduler`, started from boot)
  runs approved `every` sources once they're due (`due_sources`, which is
  pure and tested). Unapproved sources never run unattended.
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
  the `v_change` row, computing first if needed. It diffs the endpoints
  (`endpoint_diff.rs`, shared with the `diff_endpoints` IPC), reads the
  first 200 changed files' contents, runs `code_analysis::analyze_files`
  (tree-sitter metrics per side, churn, import deltas), and builds rows:
  - files: status, +/−, zone (project zone rules), `is_test`, and the
    "look here first" `interest` score + reasons (`file_interest`, ported
    from the old UI formula: `(1 + log2(1+lines)) × (1 + 0.6·Σcomplexity↑) ×
    (1 + 0.4·Σparams↑) × (1 + (longest new fn − 60)/40)`);
  - functions: added / deleted / modified (signature and/or body), deltas,
    churn and churn share; unchanged ones aren't stored;
  - imports: added/removed with zones, `cross_zone` for new boundary
    crossings;
  - co-change: `analyze_surprise` over a history cached per (repo, HEAD);
  - test files (`v_change_test_file`, V83): for each changed file that is
    a test file or has tests on either side (Rust's inline `mod tests`
    counts), test functions plus assertion calls and skip markers
    before and after (`test_signals::count`, a heuristic across
    languages; `//` and `#` comment lines don't count, so commenting an
    assertion out lowers the count).
- **Duplicates** come later: a background whole-tree scan scoped to the
  changed files (`duplication_scan::DuplicationRecorder`) stores
  `v_change_duplicate` and emits `ChangeAnalyzed` again. The same scan is
  recorded as a code-quality scan (`v_code_quality_scan` /
  `v_code_quality_finding`). It writes **no** `oxplow.duplicate_lines`
  facts: only a full-tree scan may restate that metric (tsk365,
  [code-quality.md](./code-quality.md)).
  Closed efforts (snapshot heads) get none: snapshot trees aren't
  scannable yet.
- **Caching.** A change is keyed by (stream, kind, target) — commit shas are
  resolved to full ids, so `HEAD` and a short sha share one row. Commits
  and closed efforts are computed once. Working-tree and open-effort
  changes are recomputed when stale: `spawn_invalidation` bumps a
  per-stream generation on snapshot and git-ref events and (debounced
  1.5 s) emits `ChangeStale { stream_id }`, so a page showing one calls
  `ensure_change` again. Concurrent requests for the same change return it
  `running`; `ChangeAnalyzed { change_id }` fires when results land.
- An effort without a start snapshot is an error, not an empty diff.

Deleted / skipped tests and removed assertions are `v_change_function`
(deleted `is_test` rows) plus `v_change_test_file`, read by the
oxplow-review Tests Weakened lens. Missing co-change is
`v_change_co_change`.

**Still target:** network enforcement off macOS; AI-role columns.
(Extension-declared metrics and dimensions, fact and entity, are current:
see extensions.md.)

## Relation to other docs

- [metrics.md](./metrics.md): the fact substrate this generalizes. Still
  authoritative for facts, captures, the cube and gauges.
- [extensions.md](./extensions.md): how lenses and extensions consume this
  layer.
- [ai-providers.md](./ai-providers.md): the AI functions sources and lenses
  can use.
- [collection.md](./collection.md): test-run and coverage ingest, which
  becomes the tests & coverage source.
