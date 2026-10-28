# Semantic layer

This doc covers the layer that **all oxplow data** flows through: the
sources that produce data, the dimensions that slice it, the metrics that
aggregate it, and the read-only SQL contract (`v_*`) that views, extensions
and agents query.

> **Status: partly built (epic tsk275).**
> - **Current:** the `v_*` read contract for core data, plus `oxplow_query_sql` and
>   the catalog as models (`v_model*`), and the **fact substrate**
>   (`measure`, `dimension`, `metric_spec`, `metric_capture`, `fact`, cube),
>   documented in [metrics.md](./metrics.md).
> - **Current (tsk289):** extension `exec` sources that bring external
>   entities in (see "User and extension sources").
> - **Current:** decisions and claims (`v_decision`, `v_claim`, MCP
>   `oxplow.effort.record_decision`; claims read from the agent's answers by
>   `oxplow.effort.replace_inferred_claims`), the agent-activity views, stored
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
> - **Current (tsk325):** Settings → Data: every entity with its owner and
>   row count, and the source rows. `v_data_entity` (a live model) lists
>   models and unsynced entities without counting; a row's count is read by
>   pinning its `name`, so a model too big to count within the 5 s timeout
>   costs only its own cell (tsk1065).
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
- let users and their agents build the **pages** they actually want
  ([extensions.md](./extensions.md)).

A page can only be as good as the data it can reach. So **nothing may be
reachable only through a bespoke IPC**. If core has it, the semantic layer
exposes it.

## The three pluggable parts

### 1. Collectors produce data

A **collector** (the target design called it a *source*) records one of:

- **entities**: typed rows with a key and relations (a task, a commit, an
  LSP diagnostic, a GitHub PR);
- **facts**: atomic measurements on the fact substrate (a function's
  complexity, a test case's outcome, a token count); or
- **records**: a test, coverage or analysis report it parses (the
  project's report collectors, tsk863).

Extensions (oxplow's own code metrics are oxplow-foundation's) and the project
declare collectors in the same format. A collector runs on its `on`: `manual`
(the default), `{ every: <dur> }`, `[<event types>]` (with `where`,
`settle`, `at_most`, `idle`, `force` beside it on the entry) or, for a
report collector, `{ run: test | analysis }`. An effect's `on`/`where` are
the same grammar, read by one parser and one matcher
(`collectors::parse_trigger`, `Trigger::fires`); the old nested
`trigger:` key is refused, its refusal naming the new spelling
(`oxplow_domain::manifest_keys`). Its runtime is `exec`, `starlark`, `jaq` or
`read` (`crates/oxplow-config/src/collectors.rs`; "Collectors" below).

### 2. Dimensions slice data

A **dimension** is a SQL expression or a join over an entity. Examples:
`task.priority`, `file → package`, `file → owner via CODEOWNERS`,
`pr.author`, `issue.cycle`. A dimension can be declared in any
extension (the project's own included) or the person's global files. It applies to every metric whose entity has
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

- `count(v_work_item) where state = 'done'`
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
| work | `stream`, `thread`, `task`, `task_note`, `task_link`, `effort`, `effort_file`, `decision`, `claim`, `context_read` | cycle time, steering, lifecycle |
| collectors | `collector` (each enabled extension's collectors with their trigger, enabled flag, state and last run: a plain view over `contribution`, `collector_run` and `contribution_health`; the machine's network and credential standing are the live `collector_state` / `collector_credential`) | — |
| knowledge | `wiki_page`, `comment` | freshness |
| git | `commit`, `commit_file`, `commit_work_item`, `branch`, and change analysis (`change`, `change_file`, `change_function`, `change_import`, `change_duplicate`, `change_test_file`) | churn |
| snapshots | `snapshot`, `snapshot_op` | — |
| lsp | `diagnostic` | diagnostic counts by severity |
| tests & coverage | `test_run`, `test_case` | coverage, pass/fail |
| code metrics | `function`, `file_metric` over `tree_fact` (the current tree of every per-path measure — the engine's fold) | complexity, length, params, TODOs, doc coverage |
| agent | `agent_turn`, `tool_call`, `token_usage`, `agent_nudge`, and agent events (hooks, sessions) as `v_event` `agent.*` | tokens |
| ai | `ai_call` | tokens, latency |
| usage | `page_visit` | — |
| ui | `op_error` (operations that failed in front of the person, from `ui.op_failed` events) | — |
| metrics | `measure`, `dimension`, `metric_spec`, `metric_catalog`, `capture`, `fact`, `effort_metric_delta`; series through `metric_grid()`, offenders through `metric_findings()` | — |
| the log and the registry | `event`, `event_content`, `event_checkpoint`, `event_dead_letter`; `model`, `model_column`, `model_lineage`, `model_test` | — |

Live-only for now — read over LSP or git when asked, not stored, so no
model: blame lines, and LSP references. A collector that
stores them would add their models. Decisions, claims, context reads and
struggle signals (the bundled review policy's `v_oxplow_bundled_struggle`, over `v_tool_call`) are what a person reviewing agent work needs most —
**exceptions and decisions**, not trends and totals.

## Models (P4.2)

Every connection registers three scalar functions for URIs
(`oxplow-db/src/sql_functions.rs`): `uri(resource, key1, value1, …)` builds
a canonical URI — the params sorted and escaped, a NULL value left out —
so a model or page links to another page
(`uri('oxplow/page', 'page', 'oxplow-bundled/job-run-output', 'job_run', uri)`)
or builds a record's URI (`uri('oxplow/effort', 'id', json_extract(e.payload, '$.effort'))`) without
concatenating a prefix; `uri_param(uri, key)` reads one param back (NULL when
the URI lacks it or isn't one) and `uri_resource(uri)` its resource
(`oxplow/work_item`). Other URIs are selected from their owning model's
`uri` column (`.context/resources.md` "Using URIs").

Extension models and pages may use `import_ref('binding')` for a model declared
in the extension's explicit public `imports`. The host resolves the canonical
target and exact native version before exposing the contribution; an unavailable
optional binding withholds the referring model/page. `models::import_references`
uses the shared SQL tokenizer, preserving comments, literals and quoted names.
Model calls become ordinary `ref('<extension>/<model>')` before compilation, so
lineage and model contract enforcement stay in the existing compiler. Pages
receive the quoted public view name. Authoring and impact overlays resolve a
fresh candidate catalog before compiling all enabled model definitions and
entity stubs. They use that snapshot throughout the overlay and report candidate
and declared transitive-consumer compilation failures. This does not publish
models or pin a production runtime generation. Imports never grant SQL access. See
[extensions.md](./extensions.md) for visibility and dependency propagation.

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
  version refuses to open. Every core model was reset to v1 with the
  migration squash (tsk1081): existing databases had their
  `model_contract` rows cleared, and the golden below started over.
  Because that check needs a database that recorded
  the version, the core models' contracts are also **pinned in a golden**,
  `crates/oxplow-db/fixtures/model_contracts.json` (`{ name: { version:
  columns } }`, every version published): `every_core_model_contract_is_
  pinned_at_its_version` fails in CI on a change at a pinned version, and
  `OXPLOW_BLESS=1` pins a new one (earlier versions stay; it never
  rewrites a pinned version, so a bless leaves such a change red). A sweeping
  rename never edits the golden: a published version's docs stay as they
  were, and the models whose docs the rename rewords take a bump;
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
`ModelDecl` as `models.yaml` (name, version, description, columns, key,
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
one pass after the core ones — in `Database::transaction`, which takes the
write lock up front and retries `Busy` (tsk977: run on a raw connection,
the boot pass read first and failed with "database is locked" when
another boot writer committed meanwhile, and a fresh project never got
its extensions' models until an extension changed): the last pass's extension views go, the
rest publish in dependency order, each in a savepoint — a model that
fails (resolution, lineage, contract, a name that belongs to core or an
entity), and every model reading it, is left out and reported for its
extension; a cycle is reported for each model in it. Only the primary
worktree's extensions compile (views are project-wide, like source
data). `ExtensionModelsService` runs a pass at boot and after a
deployment change, a config change, or a registry change (a
source syncing a new entity); its fingerprint — the enabled extensions'
model sources plus the registered entity views — makes a pass over the
same inputs, including the one its own registry writes set off, a no-op.
**Candidate model replacement.** `models::replace_extensions` supplies a
stricter transaction helper: compile the entire candidate inside its own
savepoint, then retain it only when every extension's diagnostic list is empty.
Any compilation or declared model-test diagnostic returns
`TxError::Aborted<ExtensionModelErrors>` and rolls back candidate views, registry
rows, contracts, model-test records, materialization changes and orphan cleanup.
Storage errors roll back too and stay typed. It does not commit the caller's
transaction; a coordinator must propagate rejection to roll back its other DB
registrations. `Database::replace_extension_models` wraps this in the normal
retried write transaction. Rejection never retries; only storage contention does.
An empty successful set removes all extension SQL models while core/entity
models remain governed by their existing rules.

`PreparedExtensionDatabase` in oxplow-app now captures the model set and
vocabulary from one CatalogSnapshot. Its transaction helper wraps strict model
and vocabulary replacements in an outer savepoint, so a vocabulary diagnostic
also restores successfully compiled models, contracts and materialized data.
It also restates enabled contribution descriptors. With prepared capabilities,
capability rows and switch events join that same savepoint and vocabulary uses
the planned work-list recognizer. Mismatched catalog identities are rejected.
It makes no outer commit or runtime swap. The returned vocabulary can be exposed
only after committed, coordinated generation publication.

This API does not change the existing partial-load background compiler: it
continues to use `compile_extension_models`. The full generation coordinator
still needs to prepare all registries, decide diagnostic compatibility and
intentional disable/removal policy, publish together and pin compatible reads.
No rollback or concurrency tests were added or run in this slice.

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
extension's health: `list_extensions` (IPC) merges them into
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
open (`oxplow extension check`) uses what the last open compiled.

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
model over it. `extension check` (the read-only `Pass::Check`) creates the
temp view only, never the table; a table no published model reads goes
after the extensions' pass (`drop_orphaned_tables`).

**Recomputing** is the asset runner's ("Assets"): `Assets::sync_models`
keeps one `SqlModelMaterializer` per materialized model — its inputs are
its `model_input` followed through live models down to tables (and to a
materialized input's own table) — re-run when the registry (`model`,
`model_input`) changes. One whose SELECT, contract or input tables
changed is re-registered, so its first recompute fills it: a contract
can change under an unchanged SELECT (an input's column changed type),
and the compiler then recreates the table empty (tsk727). A recompute refills the table, whole, in one
transaction (`DELETE`, then `INSERT … SELECT`), after its inputs have
been quiet for the coalesce window, and is recorded in `asset_state`.
**Lineage for subscriptions** routes through the table: a materialized
model changes when `m_<view>` is refilled, not when its inputs move, so
`ModelsChanged` names it and its readers once per recompute. Its
watermark is therefore when it was computed; `v_model` (v3) adds
`materialize`, `computed_at` and `events_to` (from `asset_state`).
**Keys** (P8.B1). A model may declare `key: [col, …]` — the columns
whose values name one row. Each must be a declared column, named once
(else an error at the declaration's `file:line`). The key is part of the
**contract**: `model_contract.key_json` (V141; every version recorded
before it promised none), the golden pins `{ columns, key }` per version,
and adding, dropping or reordering a key at a published version is a
change that needs a bump (`its key became [..]`). A key implies a test,
`key(<cols>)`, run before the declared ones: its columns are never null
and no two rows share them. A keyed materialized model's `m_<view>`
takes the key as its `PRIMARY KEY` (a changed key recreates the table
for the next recompute). `v_model_column` (v2) shows each column's
`key_part` (its 1-based place in the key).

**On a clock** (P8.B2). `materialize: { every: 1h }` (the collectors'
duration grammar: minutes `15m` or hours `2h`; anything else is an error
at the declaration's `file:line`) stores the model like `on_change` but
recomputes it on that clock, ignoring its inputs' changes — for SQL whose
answer moves with time (`'now'`, "the last 180 days") rather than with a
write. The runner wakes at the persisted `asset_state.computed_at +
every` (at once when it never ran or is overdue), so a restart doesn't
rebuild a fresh one; a contract change recreates the table and drops its
`asset_state` row, so the next recompute is its first build. Each
recompute records the asset's `definition` (a model's SELECT, hashed —
`Materializer::definition`, V151), and a record of another definition
doesn't hold the clock: an edited SELECT rebuilds at once rather than
serving the old SQL's rows until the clock comes round (tsk780).
What a recompute records and what the clock waits on are one clock,
`Assets`' `Now` (`Timestamp::now`; tests pass one that follows tokio's
paused clock with `with_now`, so the `every:` tests run hours of virtual
time instead of sleeping, tsk794). `model.materialize` records `every 1h` (V142 rebuilt `model` for the
wider CHECK); `v_model` is v4. oxplow-bundled's `co_change_pair` is
`every: 1h`.

**Incremental** (P8.B4). `materialize: { incremental: <column> }` keeps
a model by appending the rows past its watermark — the highest value of
`<column>` its table holds — instead of refilling it:
`INSERT INTO m_<view> SELECT * FROM (<sql>) WHERE <column> > :watermark`.
It's for append-only inputs, and the compiler holds it to that, each
refusal at `file:line` and pointing at `on_change`: **`<column>` is its
key** (`key: [<column>]`, the table's primary key) — one declared
INTEGER, unique per row, like a row id or a seq. A watermark two rows
shared would let the second slip under `>` unseen, since nothing reaches
the primary key to catch it (tsk777); a unique one can't. Its
SQL is a filter and projection over **inner joins** of append-only
inputs: it may not group, de-duplicate, window, cap or combine sets
(`GROUP BY`, `DISTINCT`, `OVER`, `LIMIT`, `UNION` / `INTERSECT` /
`EXCEPT`, an aggregate call — `json_group_*` included), outer-join
(`LEFT` / `RIGHT` / `FULL JOIN`), use a subquery (`EXISTS`, `IN (SELECT
…)`, a scalar one, a CTE — any `SELECT` after the first), or read the
clock or randomness (a date function, `CURRENT_*`, `'now'`, `random()`).
An outer join or a subquery makes a row already appended depend on rows
that land later on another input, which no append re-emits and nothing
at run time notices (tsk778). At run time it refills whole instead when it must:
- an input was **rewritten** — an UPDATE or DELETE, not only inserts
  (`Changed.rewrote`, P8.B3; an event payload expiring is an UPDATE);
- it's the **first build** since it registered (an open, a changed
  SELECT or contract — which also recreates the table);
The flag is sticky until a recompute succeeds. An append that **hits
the primary key** isn't a reason to refill: with the watermark the key,
only SQL that emits one key twice can hit it, and a refill of the same
SQL fails the same way — it's a failure (tsk781).

**A failed recompute is visible** (tsk781). Any recompute that fails —
a keyed model's SELECT emitting one key twice, a query error — leaves
the last good rows in place, keeps its retry flag, and is recorded in
`asset_failure` (V152) until a recompute succeeds: `v_asset` (v3) shows
its `failed_at` and `error`. (A materialized model's `key()` test reads
its table, which by construction holds each key once — so a duplicate
key shows as the asset's failure, not a failed test.) `extension test`'s
incremental check reports an append that fails on its fixture. A row that appears
*below* the watermark is the case nothing at run time can see; `extension
test` checks each incremental model against a full refill (P8.B5).
`asset_state` (V143) records each recompute's `mode` (`full` /
`incremental`), the `watermark` it reached and the `row_count`;
`v_asset` (v3) and `v_model` (v5, `mode`) show them.

## Metrics in SQL (P4.5)

A query reads metrics as columns of a grid:

```sql
SELECT bucket, "oxplow.language", MEASURE('oxplow.coverage.abs_pct')
FROM metric_grid('day', 'oxplow.language')
```

(The dimension's column is named by its key; a conformed key has a dot,
so the query quotes it.)

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
- The series are scoped to `SqlQuery.stream` — a page passes its
  `:stream_id`.
- **`metric_findings('<key>'[, <capture id>])`** (P4.8,
  `crates/oxplow-app/src/metric_findings.rs`) is the same mechanism for the
  located items behind a metric: a table of `subject_kind, subject,
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

## Live models

A few published models hold state that has **no change hook** to restate it
from: whether a key is in the keychain (`v_ai_provider.key_set`,
`v_ai_role.key_set`), `ai.yaml` and `project.yaml` (a person edits them by
hand), which language servers run and whose binary is on disk
(`v_lsp_server`), and a machine's collector standing and credentials
(`v_collector_state`, `v_collector_credential`; per machine, outside the
repo), what the zone table does to the worktree (`v_zone`,
`v_zone_unmatched`: `project.yaml` edited by hand, and the files on disk),
every setting's effective value (`v_config_value`), a worktree's files
(`v_workspace_file`), the harnesses' saved conversations
(`v_agent_conversation`: files the harness writes outside oxplow), the
registered agent harnesses (`v_agent_harness`: which `agents:` enables, and the settings each declares), the
repository's working copies (`v_vcs_workspace`) and how far two revisions
have parted and the commits between them (`v_branch_divergence`,
`v_commit_between`: a graph walk over refs a fetch can move, narrowed to
the pair the query pins — one side pinned, the other ranges over every
branch; neither, every local branch against the default) and every
metric's series (`v_metric_point`: the engine computes it, one row per
capture, narrowed to the `metric` and `dimension` the query pins). A restated table (as `v_extension`, `v_page`, `v_page_answer` and `v_page_param` are, in the
extension-generation transaction) would be wrong between the change and the
next restatement, and nothing says when that is. So these are **live
models** (`crates/oxplow-app/src/live_models.rs`, `LiveModels::NAMES`): the
rows are read when a query names the model.

- **How** (the `metric_grid()` mechanism, in the SQL gateway): before it
  takes a connection, `SqlGateway::materialize` finds each live model the SQL
  names (`v_<name>`, bare or quoted), reads its rows from the registry the app
  bound (`LiveSources`: the AI service, the LSP session manager, the
  collector listing, the config, the worktree's file list, the stream
  store, the harness registry and the VCS), and hands them to the query as a temp table
  `live_<name>` with a temp view `v_<name>` over it. The temp view shadows
  the core view of the same name for that query alone, and both are dropped
  with it.
- **The core model is an empty typed stub** (`SELECT CAST(NULL AS TEXT) AS …
  WHERE 0`): it carries the contract (columns, docs, version, `v_model`) and
  gives a `check` something to compile against. A gateway with nothing bound
  to read the sources (a bare `SqlGateway::new`) **refuses** a run naming one
  rather than answer empty, and a test pins that no core model is built on a
  live one (it would read the stub).
- **Stream-scoped:** a query runs for a stream (`SqlQuery::stream`, which a
  page sets from its `stream_id` param). A model of a worktree —
  `v_workspace_file`, `v_agent_conversation` — reads that stream's, and the
  primary checkout's when the query names none; its rows carry the
  `stream_id` they list. `LiveModels::materialize` takes the stream.
- **Narrowed by the query:** a model can name the columns it reads
  narrowed by (`live_models::pushdown`). When the query certainly keeps only
  rows with `<column> = <value>` (`sql_tokens::equality_filters`: one plain
  `SELECT` reading the model once as its only source, the condition a
  top-level `AND` term, a literal or a parameter — an `OR`, `BETWEEN`,
  `CASE`, join, compound, top-level `WITH` or second read of the model gives
  none; a model read once inside a parenthesised `SELECT`, a CTE body or
  a subquery, is narrowed by that `SELECT`'s own conditions),
  the read gets the value and returns only those rows; anything else reads
  them all, which is always correct. `v_workspace_file` narrows by
  `parent`: `WHERE parent = :path` is one `read_dir`, not a walk. Both go
  through `workspace_files::list_workspace_paths`, so a folder's rows are
  the walk's rows for it (an ignored folder, or one under one, has none).
- **Made on demand (stored models).** The stored-model twin of
  "narrowed by the query" (`on_demand.rs`): a model names the columns
  that, all pinned by a query's certain `<column> = <value>` conditions
  (`sql_tokens::text_filters`, the same rule; a value may be parts joined
  by `||`, each a literal or a parameter — `Operand::Concat`, how a range
  change's `target` is named from its two sides — and is certain only
  when every part is a text), ask for one thing; the
  gateway has that thing stored before the query runs, when a run (not a
  check) asks for it. `v_commit` and `v_commit_file` by `sha` (the full
  id): `commit_indexer::ensure_commit` runs the workspace's scan first, so
  a commit not yet reached is new history, indexed and logged
  (`vcs.commit.indexed`, which lands efforts); one still missing is past
  the horizon, stored and linked but not logged; one git doesn't know
  reads no rows. `v_change` by `kind` and `target`
  (`change_analysis::request`): the row is created now and its analysis
  started in the background when the row is `pending` or its head moved,
  so the query reads the change at once and what reads `v_change_*` reads
  again when the analysis commits. A page names what it shows this way
  (a commit document's `query:`), and so does an agent's `query_sql`. A
  failure to make it is logged; the query reads what is stored. A gateway
  with nothing bound (a check) makes nothing.
- **Cheap, and current:** a read walks in-memory registries, a config file,
  the keychain, the stream store, a worktree's files (`v_zone`, and
  `v_workspace_file` unless the query names a folder, walk them on every
  query) or each harness's conversation index;
  it holds no lock while the query runs. Nothing is cached, so a result is
  as current as the query. A live read has no watermark, so a result's
  `freshness` doesn't mention it.
- **Changed, as any model is:** `live_models::follows` says what changes
  each one — the events about the state it reads (`ConfigChanged`,
  `WorkspaceChanged`, `VcsRefsChanged`, `LspServersChanged`,
  `CredentialChanged`, `ExtensionsChanged`) and the
  models it reads through (`v_agent_conversation` follows `v_agent_session`
  and `v_agent_turn`; `v_vcs_workspace` follows `v_stream`;
  `v_branch_divergence` and `v_commit_between` follow `v_branch`, the
  branches an unpinned side ranges over). The bus, as
  it posts such an event, posts the `ModelsChanged` naming the live models
  that follow it (`models_changed::announced_after`, so every emitter is
  covered and no backend code listens to the bus) and tells the open page
  runs the same models (`EventBus::reach_open_runs`, set once as the
  services are built — the change loop sees only tables, so without it a
  keychain write never re-ran the page showing it). The change loop adds
  the live models that read through the models a commit changed
  (`live_models_reading`). A page over a live model then re-runs like one
  over a table: its `reads.models` names it.
- **Never a secret:** the keychain is asked whether an entry exists
  (`key_set`, `is_set`), never for its value; a test stores a key and a
  credential and proves neither reaches any column.
- **Adding one:** a stub `models/<name>.sql` and its `models.yaml` entry (with
  column docs), the name, columns and what it `follows` in `live_models.rs`,
  a read in `LiveSources`, and an answerability question. Declare an integer column
  `INT` (what SQLite infers for a `CAST`). If the state does have a change
  hook, restate it in a table instead.

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
  hearing it sees the change: `Database::subscribe_changes()`. What's
  published is a `Changed { tables, rewrote }` (P8.B3): `rewrote` names
  the tables the commits updated or deleted rows in — not only inserted
  into — which is what tells an incremental model that appending isn't
  enough.
- **Models:** `crates/oxplow-app/src/models_changed.rs` follows
  `model_input` from those tables to every model that reads them, directly
  or through other models (reloading the lineage when `model_input`
  itself changes, and treating a lagged channel as "everything changed" —
  every table touched, the registry included: one `react` for both, so a
  missed batch that held a `model`, `model_input` or `resource` write
  still re-registers the materialized models and the searchable resources'
  indexes, tsk854),
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
- **Pages** run on the daemon as open runs (`crates/oxplow-app/src/pages/open_runs.rs`):
  the change loop signals them as it tells the assets, a worker coalesces
  signals for 100 ms and re-runs each open run one of whose loaders read a
  changed model or measure (an empty list is "unknown", so everything
  re-runs; a command loader's reads are its declared `reads`), and the
  window fetches what changed. A change announced while a run was in flight
  is remembered and re-runs it once the run's `reads` match it — a write made
  while the window loaded would otherwise never show.
- **Other UI hosts** (the rail's alerts, the explorer) keep their last run's
  `reads` and re-run through `useRerunOnChange` when `modelsChanged` names a
  model they read, `metricSamplesChanged` names one of its measures, or
  `extensionsChanged` says the extension catalog changed (the daemon relays
  the catalog's own signal: a change to the live deployments or the
  `extensions` key; a file path is not the signal). Hosts that list what
  extensions contribute (`extensionsStore`, the launcher, what pages
  answer) reload on it, on `configChanged`, and on a daemon reconnect (a
  change while it was unreachable sent nothing). A burst of commits
  coalesces into one re-run (100 ms).

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

**A restart rebuilds only what changed.** Which tables a commit touched is
known in memory, so without more every asset would rebuild at boot to be
safe. Instead each table an asset reads or writes (`outputs()`, a
materialized model's own table) carries triggers that bump its row in
`table_generation` inside the writing transaction, whatever path wrote
it (`oxplow-db/src/table_generations.rs`, V11). A recompute records
what it was built from in `asset_state.built_from`: this build of the
program (the executable's identity), its `definition()`, its inputs'
generations as it began and its outputs' as it ended. At registration an
asset whose stored `built_from` still matches skips its first build. A
table without the triggers (dropped and recreated, never tracked) is
unknown, so it never matches; nor does a new binary.

An input may name **another asset** instead of a table: the asset then
recomputes after that one does — each recompute the other finishes marks
it dirty — and never while that one is pending (recomputes asked for but
not yet done); its first build waits for the other's. `effort_evidence`
reads `metric_cube` this way rather than `metric_capture` and `fact`, so
its series reads are cube-served: racing the cube's backfill at startup,
every one missed and folded a measure's whole history (gigabytes).

Derived data whose inputs include the world **outside** the tables (a
working tree, the VCS tree, a program, an external provider) is ingestion
instead — a collector or a pump consumer, at-least-once and checkpointed.

A **snapshot blob named by a table row** counts as a table input (tsk862):
content-addressed, it never changes under its name, so a recompute after the
row lands reads what any later one would. `effort_evidence`'s diff coverage
reads the blobs of the effort's start snapshot and of the snapshot a
coverage capture is pinned to. A blob can only go away (collected), and the
asset then says so on its next recompute — the diff gives no row — rather
than reading something else. A working tree is never such an input.

The metric cube is the first asset (inputs `metric_capture`, `fact`; read
by `effort_evidence`): see
[metrics.md](./metrics.md). What a burst costs it is in
[performance.md](./performance.md).

## The `v_*` contract (current)

Every shipped entity is exposed as a stable **read-only SQL view**. They
are the **versioned contract**: pages, extensions and agents read these,
never the physical tables, which stay internal and free to change.

**Shipped today** — each a core model, `crates/oxplow-db/models/<name>.sql`
(see "Models"); the version history in the parentheses is the migrations
that made them before P4.2. `v_model`, `v_model_column`, `v_model_lineage`
and `v_model_test` are the catalog of all of them:

| View | What it is |
|---|---|
| `v_stream` | streams (worktrees) |
| `v_thread` | threads within a stream |
| `v_knowledge_touch` | which threads wrote which knowledge pages, and when each last did (`page`, `thread_id`, `last_seen_at`; the rail's Finished section, P6.E1b) |
| `v_knowledge_page` / `v_knowledge_ref` | knowledge pages with their outbound refs and `stale_ref_count`, and each page's file refs with their pin and `stale` (P5.C4; knowledge.md) |
| `v_work_item` | the active work list's items (`uri`, `provider`, `native_id`, canonical `state`, `native_state`, `native` JSON, `parent_uri`), excluding deleted; everything reads it — oxplow's tasks have no model of their own (`v_task` was removed) and keep their own fields in `native_state` / `native`; oxplow's tasks are `work_item:oxplow:tsk<n>` (V115, P5.C1; data-model.md) |
| `v_effort` | spans of a thread's work, linked to a work item or not (`work_item` URI; join `v_work_item` for the item); `title` defaults to the item's or the first prompt's, `summary` (v3) to the last turn's final message, and `closed_by` says what closed it |
| `v_comment` | comment threads, with first-message `body` and `message_count` |
| `v_comment_message` | each message of a thread, by `comment` (the thread's URI) and `position` from 1, with `author` and `body` |
| `v_wiki_page` | wiki pages (excerpt; full body is on disk) |
| `v_snapshot` | worktree snapshots, with `tree_hash` (whole-tree identity, V96) |
| `v_snapshot_file` | every captured version of a file (`file_snapshot`): its snapshot, stream, path, storage class, size and `content_hash`. A snapshot records only what changed since the stream's previous one, so a file's history is its rows, and `kind` (v2) says what the snapshot did to the path: `added`, `modified`, `deleted`, or NULL when the row changed nothing (P11, tsk944) |
| `v_snapshot_op` | the snapshot operation log: one row per take (a new snapshot or an unchanged tree) with `parent_snapshot_id`, `trigger`, thread / turn / effort anchors, `elapsed_ms`, `budget_ms`, `over_budget` (V97) |
| `v_measure` | fact-type catalog |
| `v_capture` | the scan/run that produced facts |
| `v_fact` | atomic measurements, joined to `measure_key` and capture context |
| `v_effort_file` | files each effort changed, with change kind, `source` (`claimed` by an edit tool or `observed` changing in one of its thread's turns, v2) and the effort's `work_item` |
| `v_work_item_comment` | comments on the active list's items (`v_task_note` was removed) |
| `v_handoff` | work handed from one thread to another (`oxplow.handoff.*`; V84): sender thread and session, recipient thread, work item, body, `delivered_at` (stamped at the recipient's next prompt), `state` `open` / `taken` / `declined` / `withdrawn`; `v_search_handoff` is its search feed |
| `v_work_item_link` | typed links between the active list's items (`v_task_link` was removed) |
| `v_agent_turn` | human prompt → agent answer, per thread (V74), with the snapshots the turn started and ended at (`start_snapshot_id`, `snapshot_id`: what the turn changed; V98/V99) |
| `v_token_usage` | model tokens per thread / effort / model, with each turn's prompt (V74, `prompt` V82) |
| `v_page_visit` | pages the human opened, and for how long (V74) |
| `v_event` | the event log: every activity and state change, oldest first by `seq`, with anchors, subject URIs and payload (V94; see [data-model.md](./data-model.md) "event_log"); `payload_expired_at` since V102) |
| `v_extension_run` | one row per finished run of extension code (V48; [data-model.md](./data-model.md) "extension_run"): `run_id`, `parent_run_id`, `extension`, `contribution`, `kind`, `version`, `generation`, `actor_kind`, `actor`, the anchors, `cause_event_id`, `mode`, `started_at`, `finished_at`, `outcome`, `error`, `wall_ms`, `cpu_ms`, `peak_bytes`, `ai_calls`, `scopes`, `decisions`, `latest` (1 on a contribution's most recent run). `v_effect_run`, `v_collector_run` and `v_ai_call` name theirs in `run_id`. Kept 90 days |
| `v_job_run` | each job run — one execution of a job an extension declares (V56; [extensions.md](./extensions.md) "Jobs"): `id`, `job` (`job:<ext>/<id>`), `uri` (`job_run:<id>`), `extension`, `version`, `contract_version`, `operation`, `state`, `attempt`, `cancellation_requested`, `invoker`, `run_id`, `created_at`, `updated_at`, and its current attempt's `ai_calls`, `tool_calls`, `input_tokens`, `output_tokens`. Never its request or result |
| `v_job_run_output` | what a job produced as it ran, in order (`job_id`, `attempt`, `seq`): `kind` (`text`, `tool_call`, `tool_result`, `usage`), `at`, `text` (a text row's), `data` (the row as JSON). Written as it streams, so a page over it follows a job live |
| `v_job_run_observation` | what a job's worker said after its attempt was fenced (a lease recovered, a permission removed, a declaration changed): one row per late result (`id`, `job_id`, `attempt`, `fence`, `at`, `outcome`, `evidence`). Evidence only; it never changes the job |
| `v_op_error` | operations that failed in front of the person in the app, one row per `ui.op_failed@1` event (`oxplow.ui.report_error`, tsk1072): `event_id`, `at`, `stream_id`, `thread`, `label`, `command`, `message`, `exit_code`, `signal`, `duration_ms`, `output_size`, and the person's marks `seen` / `dismissed` (v2: `ui.op_error.seen` / `ui.op_error.dismissed` events, from `oxplow.op_error.*`; the Alerts page lists those not dismissed). The stderr / stdout are the event's `output` body (`oxplow_read` `event:<id>#output`). Kept 30 days, the output 14; an expired event keeps its row with its details NULL and `payload_expired_at` set |
| `v_op_error_output` | what each failed operation printed: `event_id`, `stream_id`, `stderr`, `stdout`, `truncated` — its `ui.op_failed` event's `output` body, read in SQL from `event_content`'s bytes (stored as JSON text). The one event body published to SQL: it is what the person saw on screen. Tool input and output and prompts stay behind `read_event_content` (`v_event_content` lists them without bytes). A failure with nothing captured, or whose body retention removed, has no row |
| `v_event_content` | large event bodies (tool input/output, prompts) by content hash, without the bytes (V102) |
| `v_event_dead_letter` | events a consumer failed on, parked with the error; `pending` ones need `oxplow.delivery.retry` / `oxplow.delivery.discard` (V94) |
| `v_event_checkpoint` | how far each event consumer has read (V94) |
| `v_decision` | forks the agent resolved (question, choice, alternatives, confidence, why). `provenance`: `recorded` via the `oxplow.effort.record_decision` command (V76; P8.A7), or `inferred` when the effort closed (V79) — a recorded `extract` computation on the `main` role (`AiCompute`, so an unchanged effort doesn't call again) — and a reviewer's verdict on an inferred one: `confirmed` or `dismissed` (V132, `oxplow.effort.confirm_decision` / `oxplow.effort.dismiss_decision`; a re-inference replaces only the still-`inferred` ones) |
| `v_claim` | claims the agent made in its answers ("tests pass"), as a model read them when the effort finished (`oxplow.effort.replace_inferred_claims`), what each cites (`evidence`; `reviewer` when a person verified it by looking) and a reviewer's verdict (`review`: `verified` or `dismissed`, V75). Whether anything else backs a claim is a review policy's: oxplow-bundled's `v_oxplow_bundled_claim_status` counts a `tests_pass` claim backed when its effort's **latest** `v_test_run` has `failed = 0 AND total > 0` |
| `v_tool_call` | every agent tool call: the `tool_call.project` pump consumer's projection of `agent.tool.finished` (one row per event; the call as its harness mapped it, `oxplow-app/src/tool_calls.rs`; paths worktree-relative; a shell call's `ok` is NULL when its harness reports no exit code). `tool` is the harness's own name, `kind` oxplow's (V42; v3) (V77; `turn_id`, `event_id` since V102) |
| `v_context_read` | reads (`kind = 'read'`) of `.context/*.md` (V77) |
| `v_oxplow_bundled_struggle` | oxplow-bundled's model (review policy, not core): per effort, a file edited 5+ times, or 3+ failed shell commands, by kind (`subject` `shell`), over `v_tool_call` |
| `v_ai_call` | oxplow's own model calls: role, provider, model, caller, tokens, latency, ok/error, a recorded computation's `input_hash`, and its `uri` (`ai_call:<id>`, whose bodies `oxplow_read` gives) (V78, V118; see [ai-providers.md](./ai-providers.md)) |
| `v_ai_result` | recorded AI computations (`classify` / `score` / `summarize` / `extract`), keyed by input hash, model and prompt version, with the output, tokens and the computing call (V118; [ai-providers.md](./ai-providers.md) "Recorded computations") |
| `v_metric_spec` | metric definitions: aggregation, direction, target / warn / fail (enabled lives in project.yaml) (V80) |
| `v_agent_nudge` | guidance oxplow sent the agent mid-effort (V80; `turn_id`, `delivered_at` since V102) |
| `v_code_quality_scan`, `v_code_quality_finding` | code-quality scans and their findings (duplicate blocks, with the peer in `extra_json`) (V80) |
| `v_dashboard`, `v_dashboard_item` | user dashboards and their tiles (V80) |
| `v_terminal` | a stream's terminals: shells in its worktree, with the `uri` their PTY is keyed by (V106) |
| `v_effort_metric_delta` | per effort, how each metric moved (baseline → current, `crossing`). **Stored, not a live query:** the metric engine computes it (`CollectionService::refresh_effort_evidence`) and `effort_evidence.rs` refreshes it on `effort.finished` (the `effort.evidence` pump consumer) and for every open effort as the **asset** `effort_evidence` (`OpenEffortEvidence`, P7.B6: its inputs `metric_capture`, `fact`, `effort_file` — not `agent_token_usage`, which it doesn't read: token counts reach it as facts; its own tables aren't, so it can't loop) — which also recomputes every **closed** effort whose runs or files moved since its rows were stored (`effort_evidence_state.attribution_sig`: a run collected late, an adoption's restamp); readers hear the rows move as `ModelsChanged` (V80). **A recompute reads what moved, not the effort's history** (`.context/performance.md` "Open-effort evidence"): it asks the effort's captures per producer and trigger (count, newest id), memoizes each spec on those of the producers it reads, folds a `sum` spec's new reports into a running total, and reads only the run captures for observations |
| `v_effort_observation` | per effort, test runs / diff coverage / analysis rebuilt from its own run captures; refreshed the same way (V80). `seq` runs oldest first, so the highest is the latest — every reader takes `ORDER BY seq DESC` as newest. Diff coverage is start snapshot vs the capture's snapshot (collection.md). |
| `v_change`, `v_change_file` (a `working` change's files say `staged` and `untracked`, restated by `change_reactor::watch_working_files` at boot and whenever the stream's refs or index move), `v_change_function`, `v_change_import`, `v_change_duplicate` | stored change analysis (see "Change analysis" below) (V81) |
| `v_commit`, `v_commit_file`, `v_commit_work_item`, `v_branch`, `v_tag` | history, branches and tags (V85, V113). The commit indexer (`commit_indexer.rs`, run at boot and on each ref move, `RefMoves`) stores the commits reachable from **every stream's** head — new history whole, up to a 5000-commit horizon (`IndexDepth`), and any older commit a query names by `sha` ("Made on demand") — through the VCS capability (`Vcs::log`/`revision`, `.context/vcs.md`) — with their files (first-parent diff) as it projects them into `page_ref`; `v_commit.parents` (JSON, v2) lets a stream's history be a recursive read from its branch's head. `v_commit_work_item` reads the indexer's work-item mention edges and the work-item → commit links (declared impacts, held effort work), for the active work list's items. `refresh_refs` restates `v_branch` (`is_default`, v2) and `v_tag` from `Vcs::branches`/`tags` and maps each local branch to the stream checked out on it. The desktop's history panel, dashboard lists, branch picker and new-stream form read these models (`apps/desktop/src/vcsHistory.ts`) |
| `v_test_case_stat` | each test's summary per stream, branch and producer (V139, tsk733): last status and duration, runs, failures, flips, last failed / passed, max and mean duration — updated with every run, the home of per-test history since a run writes change-only per-case facts |
| `v_test_run`, `v_test_case` | test runs (V87), views only. A run IS its `metric_capture` (producer `tests`, or `test-run` for a run that measured nothing), and both views read its verbatim `test-detail` payload in `detail_json`: counts from the payload, cases by `json_each` over `suites[].cases[]`. Cases come from the payload, not the `oxplow.test_case` facts, because those are skipped while every tests metric is disabled. `effort_id` (v2) is the capture's own: the effort of the tool call that ran it. A run that only reported counts (`oxplow.test.record_run`) has no cases |
| `v_diagnostic` | what the language servers have published, right now (V86). `lsp_diagnostics.rs` subscribes to the LSP session broadcast and replaces a file's rows per `(stream, language, path)` on each `textDocument/publishDiagnostics` (paths repo-relative, positions 1-based, a URI outside the worktree dropped). Live state: the table is cleared at boot and a server's rows when it restarts, crashes or stops. Only files a server has published appear (usually the open ones, not the whole repo). A view re-runs on `ModelsChanged` when rows commit; at most every 500 ms per stream, from the first change (so a server that publishes continuously can't starve it), `code.diagnostics.changed@2` is logged per changed file (P5.C5) |
| `v_symbol` / `v_symbol_capture` | the definitions oxplow's own tree-sitter extraction finds in each stream's files, in every supported language, no language server needed (V62) — the current tree, restated per changed file at each snapshot by the `symbols.collect` pump consumer (`uri` `oxplow:oxplow/symbol?path=<path>&symbol=<id>`, no revision, unique per stream; `kind` from one vocabulary; `container` `::`-joined; `signature` the parameter types in overloading languages; the name's `line`/`col` and the whole symbol's `start_*`..`end_*`), and what each snapshot's collection covered (`files_collected`, `files_failed`: content unreadable at the snapshot). Details: language-support.md "Symbols" |

**Column docs live with the model, not here.** `models.yaml` documents
every column, and the registry is the catalog: `v_model` +
`v_model_column`, read through `oxplow_query_sql` (extension entities too —
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
  `oxplow_query_sql`, pages, advisories, entity metrics, extension checks
  (`oxplow_validate_extension`, `oxplow extension check`) and source inputs alike.
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
    "`work_item` is a physical table, not a published model; read
    v_work_item",
    naming the models whose `source()` it is (from `model_input`), and
    never with SQLite's "no such table" (which `explain_unsynced` reads as
    an unsynced source). A table that doesn't exist at all names the
    published model it most likely meant ("no such table: v_work_items; did
    you mean `v_work_item`?", edit distance, read once the session's authorizer is
    off — tsk1039). An unknown column names the columns of each published
    model the query names, and the nearest one ("no such column: titel …;
    did you mean `title`? v_work_item has: …"), so an agent's next query is
    right without a discovery query (agents spent about one call in six
    guessing columns). `count(*)` over a view is allowed (SQLite reports
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
    { raw: true }` — the MCP tool has no such switch — and `oxplow.page.keep`
    checks a spec's query through the enforced path (`check_query_on`, as
    `oxplow.page.show` does), so a raw query can't become a page. A model's compile reads its sources in record mode. Installed before `prepare` in a `ReadSession`
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
    `truncated: true` means more rows existed. Every host that hands
    rows on says what happened past the cap — see "Completeness and
    coverage" below.
  - Caller mistakes (bad SQL, writes, timeouts) are `Invalid` errors: the
    IPC `INVALID` code, and MCP `invalid_params`.
  - `reads.tables` is empty except in a raw read. Every bundled page,
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
- Pages never call models or run collectors on render; they read what
  collectors already produced.
- Everything an extension adds (entities, relations, dimensions, metrics)
  appears in the same models (`v_model`, `v_dimension`, `v_metric_spec`) and
  the same `oxplow_query_sql` as core data. Agents never need
  an extension-specific tool. Full agent surface:
  [extensions.md](./extensions.md) → "Agents: the MCP surface".

### Completeness and coverage

A query finishing says two things it doesn't establish on its own: that
the rows are all the rows, and that the data behind them was collected.
The result keeps the two apart.

- **Rows: complete, bounded on purpose, or an error.** `truncated` is
  the gateway's flag. A host that returns rows alone never drops it:
  `sql.read` without `page` fails past 1000 rows; with `page: { limit,
  offset }` (limit 1–1000) it answers `{ rows, more, next_offset?,
  warning? }` — `more` says whether rows follow, `next_offset` where the
  next page starts, and `warning` is set when the SQL has no `ORDER BY`
  (pages of an unordered query can overlap or skip). A collector's
  `input` fails past 10 000 rows (`collectors::MAX_INPUT_ROWS`) unless it
  declares `input: { sql, limit }`; then the script gets `limit` rows
  and `input.complete` saying whether they were all of them
  (`collector_runner::input_rows_for_run`). The page view and Explore Data say
  "Showing the first N rows" above a truncated result
  (`apps/desktop/src/page/completeness.ts`, `ResultCompleteness`); a
  compact strip shows nothing. Exactly the cap is allowed; SQL
  aggregation sees every matching row before the cap, and an authored
  `LIMIT` is the query's own subset.
- **Coverage: what fed each model read, observed.** `coverage` has one
  entry per collector that declares an entity whose view the query read
  (`collection_coverage::Feeds`, restated on every catalog change):
  `{ model, source: "collector:<owner>/<id>", status, at, error }`, the
  status read at query time from `collector_run` and
  `contribution_health` — `never` (no run), `failed` (the last run's
  error), `disabled` (health says so, with its
  reason), `stale` (its schedule's `next_due_at` passed) or `current`.
  Core's own models carry no entry: core keeps them current. Nothing is
  inferred from timestamps alone. The page view lists each entry that
  isn't `current` with its last run and error.
- **Generation: what the catalog was.** `generation` is the extension
  catalog's fingerprint (`ExtensionCatalog::generation`) when the query
  ran — an opaque string that changes when any extension does.
- MCP and RPC `query_sql` and page runs return the whole result (over MCP
  also as structured content, typed by the tool's output schema); the
  row-only script reads carry the row guarantees but not coverage.
- `freshness` is separate: model changes observed since startup.
- What an agent should do with all of this when it reports numbers is the
  `oxplow-extension` skill's `reading-data` reference (aggregate before
  the cap, metric folds over naive sums, unavailable vs empty, scope), whose
  example queries `reading_data_tests` runs over a seeded project.

## Collectors (P7.B3)

A **collector** brings data in: a program (`exec`), a sandboxed script
(`starlark` / `jaq`) or a provider's read (`read`). It writes
**entities** (rows a model can `ref()`), **facts** (measurements on
declared measures) or **records** (a test / coverage / analysis report it
parses — an own extension's report collectors, tsk863), one of the three. One declaration, `oxplow_config::collectors`
(`CollectorSpec`, `parse_collectors`), serves `extension.yaml` (v1
`sources:` is migrated to it, each `schedule:` becoming an `on:`),
parsed with a `Declarer { extension, namespace, own }`; `.oxplow/project.yaml`
has no `collectors:` (it is refused). The real, tested example
is `examples/extensions/github/` (PRs from the GitHub API via `gh` or
`GITHUB_TOKEN`); the user guide is `docs/guide/pages.md`.

```yaml
collectors:
  - id: prs                    # dotted ids are fine: repo.scan_clone
    doc: The repo's recent pull requests.
    runtime: exec              # or starlark / jaq (derived, below) / read
    entry: sync.sh             # relative, inside the extension folder
    on: { every: 15m }    # or manual; { on: [<event type>], where?: {field: value} }
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
          - { to: v_work_item, on: "v_github_pr.title LIKE '%' || v_work_item.native_id || '%'" }
```

The entry prints `{"entities": {"<name>": [ {col: value, …}, … ]}}`,
and may add `"events": [{ "type", "payload", "subject"? }]` — its
extension's own declared event types, logged with the run (P9.D2;
[extensions.md](./extensions.md) "Event types").

**Entity vs fact collectors.**

- An **entity collector** (`entities:`) writes rows published as
  `v_<owner>_<entity>`; `collector_runner` runs it. It may be `exec`
  (needs `permissions.process`), `starlark` / `jaq` (derived, below; a Starlark one may call
  `ai_*` with `permissions.ai`, and read the tree when it declares `tree.read`)
  or `read`.
- A **fact collector** (`facts: [<measure>, …]`) is what a *gauge* was
  until P7.B3. It runs in the fact engine (`MetricsService`,
  [metrics.md](./metrics.md)): its Starlark gets the snapshot tree
  (`files()` / `source_files()` / `ast_query()`, `tree.read`) and, when
  it declares them, the semantic layer and a model (`permissions.ai`); its input is `{report?, rows?, event?}` (`report: { path,
  format }` parsed — text | json | xml | lcov | lines; the `input:`
  query's rows; the trigger event); it returns `{"facts": [{measure,
  value, subject?, path?, line?, rule?, num?, den?, dims?}]}` and nothing
  else (`facts_of`). A fact on a measure not in `facts:` or not defined is
  dropped. Each run records one metric capture (or a failed one) plus a
  `collector_run` row and `collector.synced@2` with its `facts` count.
  Snapshot runs keep the delta / full-baseline machinery (pending-baseline
  queue, `scan_kind`, fingerprint, dominated-capture prune).
- A **report collector** (`records: tests | coverage | analysis`,
  tsk863) is a non-bundled extension's (`Extension::is_own()`; the project's, in
  `.oxplow/extensions/project/`): it parses one `report:` file with a bundled
  parser (`entry: "oxplow:<junit|lcov|cobertura|jacoco|clippy|eslint>"`)
  or its own jaq / Starlark / exec script (paths relative to the extension
  folder; `report.path` relative to the project root), on `on: { run: test |
  analysis }` — when the `collection` reactor detects that kind of run and
  the run wrote the report — or by hand (`oxplow.collector.sync`, which records
  in a thread). The collection service runs it and merges its output into
  the run (`.context/collection.md`); its run and health are any
  collector's (`collector_run`, `collector.synced@2`, `contribution_health`), and
  its extension's permissions apply (`process` for an exec script).

**Owners.** An extension (its name), always: the project's own collectors
belong to its own extension, `project` by convention
(`.oxplow/extensions/project/`, `collector:project/<id>`), and `project` is an
ordinary extension name. There are no global collectors. oxplow's `oxplow.*` code collectors are oxplow-foundation's; one
that metrics name runs only while one of them is on.

- **The project's own extension** declares its collectors in its
  `extension.yaml`, scripts under `collectors/` (relative to the extension
  folder).
- **An extension's fact collector is sandboxed** (starlark or jaq). Only a
  non-bundled extension (the project's own) may declare an `exec` fact collector or
  report collectors, and those need the extension to declare `permissions.process` like any
  exec collector (the loader refuses it otherwise), approved when the
  extension is deployed.
- **A fact collector gets no `env` / `credentials` / `network`**, and
  `report` is for fact collectors only.

**No `gauges:`.** The retired producer block is a key neither file has
(tsk865): `extension.yaml` rejects it like any unknown key. There is no migrator.

**Running.** `on:` collectors, entity and fact alike, run from the
`collector.triggers` consumer (below). For a fact collector:
`snapshot.taken` runs a delta collector only when the take recorded
files (not `unchanged`, `file_count > 0`) and a whole-tree one on every
take — an unchanged take after a ref move has a new revision to restate; `effort.finished` runs it over the
effort's end snapshot; any other type over the stream's latest snapshot.
`every:` collectors run from the scheduler as the system through
`oxplow.collector.sync`. **`oxplow.collector.sync { collector }` is the one manual run
for every collector** — it replaced `source.sync` and `metric.run`; for a
fact collector it returns `facts`. (`oxplow.metric.rebuild` still runs the
whole-tree baseline.)

**Parse rules.** `entry` for exec/starlark/jaq; `provider: { instance,
collector }` (and nothing a script needs) for `read`, whose records land
in the capability's model and which `oxplow.provider.sync` runs; `env`,
`credentials`, `network` only on an exec entity collector; `input` only on
starlark/jaq; exactly one of `entities` / `facts`; `report` only with
`facts`; a fact collector isn't `exec` unless its extension is own;
`records:` only on an own extension; `after` and `where` only with `on:`,
whose types must be registered (`v_event_type`); `oxplow.` ids are
oxplow's own.

**An entity collector's run** (`collector_runner::run_collector`) commits its rows, its
`collector_run` row and its `collector.synced@2` event in **one
transaction**; a failed run commits the failure the same way (no rows,
the last good counts and checkpoint kept). The event's subject is
`oxplow:oxplow/collector?collector=<owner>/<id>` (resource `oxplow/collector`), its `trigger` is
`manual` (`oxplow.collector.sync` by an actor), `every` (the scheduler, as the
system) or `on`. `collector_run` (V133, `v_collector_run`; it replaced
V75 `ext_source_state`) keeps `owner, id, status (ok | error |
skipped), last_run_at, error, row_counts_json, cursor_json,
last_event_id`. The UI refreshes when `v_collector_run` changes
(`collectorRan`); the in-memory `SourceSynced` is gone.

**Health (P7.C2).** Collectors share the contribution failure policy
([extensions.md](./extensions.md) "Health, disable and repair"): three
failed runs in a row disable one (`contribution_health`, key owner / id, kind
`collector`); then nothing runs it until a person's `oxplow.contribution.enable`, and
a page over its view warns that its rows aren't refreshing.

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
  integers, and the event itself — `:event_id` its id, `:event_seq` its
  seq; NULL when absent), and the script gets the event envelope as
  `input.event` ([extensions.md](./extensions.md) "The event envelope").
- **Once per event.** The run's `collector_run.last_event_id` is the
  event's seq, and its `collector.synced@2` is caused by the event with a
  per-event dedupe key. They commit with what the run wrote — an entity
  collector's rows, or a fact collector's capture and facts
  (`RunLog::record_with`, tsk712) — so a redelivered event writes nothing,
  and no run lands without its record.
- **Failure.** A failing collector is recorded and announced like any run
  and doesn't dead-letter the event: one broken collector never holds up
  the rest. An exec collector whose extension doesn't declare `permissions.process`
  doesn't load, so it never runs.
- **Order.** A collector's `after:` names effects, collectors or core
  consumers (extensions.md "Ordering"); collectors of one event run level
  by level. An event waits on the core consumers named by the collectors
  **it triggers** (`after_for(event_type)`); a target nothing declares is a
  load error: an `effort.finished` event waits
  for `change.analyze` because `effort_churn` names it, while a
  `snapshot.taken` event doesn't (tsk711). `on: [collector.synced]` is
  refused at parse.

**Derived collectors** (`runtime: starlark` or `jaq`, tsk323) compute
entities from data already in the semantic layer:

```yaml
  - id: hot
    runtime: starlark          # def transform(input): return {"entities": {...}}
    entry: sources/hot.star
    input: "SELECT uri, title FROM v_work_item WHERE json_extract(native, '$.priority') = 'high'"
    entities: [...]
```

- **Input.** `input` is read-only SQL through the SQL gateway.
  More than 10k rows fails the run rather than deriving from a partial
  set. The script gets `{"rows": [{col: value, …}]}` and returns the exec
  shape.
- **Sandbox.** It runs in a script worker with no files, network, env or
  secrets. Its ways out are oxplow's own services, each one it declares
  (extensions.md "Services"): the captured tree (`tree.read`), the
  semantic layer (`sql.read`), its own private state (`state.read`, served by
  StateSource outside SQL), and the `ai_*` builtins (`ai_classify` /
  `ai_score` / `ai_summarize` / `ai_extract`, implied when its script
  names one), recorded computations on the project's AI roles as caller
  `collector:<owner>/<id>`, whose wait is left out of the budget
  ([ai-providers.md](./ai-providers.md) "`ai_*` functions for sources").
  A model call spends the person's key — on an `every:` trigger, every few
  minutes, run as nobody — so a Starlark script that names an `ai_*`
  builtin (`oxplow_script::ai::calls_ai`: any identifier, so an alias
  counts; Starlark reaches a global only by name) needs `permissions.ai`
  in its extension's manifest, approved when the extension is deployed: the
  loader refuses the extension without it, and the run's `ai.compute`
  decision is denied (`RunCollectorError::Denied`, recorded as the run's
  `error`) when the extension lacks it. Only a permitted script is
  handed a model at all (the services router builds no model for any
  other run, extensions.md "Services").
  Any other derived collector needs **no permission**: the scheduler runs
  it on its `every:` trigger.
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
| Fact collectors: `FactCollector`, `fact_collectors()`, `run_collector_by_key`, `run_snapshot_collectors` / `run_effort_collectors` / `run_event_collectors` | `crates/oxplow-app/src/metrics_service.rs` |
| Script services (`runtime::Hosts`, `TreeHost`), `facts_of`, `parse_report` | `crates/oxplow-script/src/lib.rs`, `runtime.rs` |
| Exec, coercion, `run_collector` / `run_for_event`, `CollectorRunner` + the `oxplow.collector.sync` command, scheduler | `crates/oxplow-app/src/collector_runner.rs` |
| The `collector.triggers` consumer (`on:` / `where` / `after`) | `crates/oxplow-app/src/collector_triggers.rs` |
| Entity tables + views, run state (V133 `collector_run`) | `crates/oxplow-db/src/collector_store.rs` |
| Settings → Data read model (`data_entities`) | `crates/oxplow-app/src/semantic_catalog.rs` |
| `v_collector` (standing) / `v_collector_credential`; setting a credential is `run_command oxplow.collector.set_credential`, running is `run_command oxplow.collector.sync` | `crates/oxplow-app/src/collector_runner.rs` |
| MCP `oxplow_preview_collector`; an agent syncs with `run_command oxplow.collector.sync`; standing and credentials read `v_collector_state` / `v_collector_credential` (the `oxplow.collector.sync` command as the agent) | `crates/oxplow-mcp/src/lib.rs` |
| UI: Settings → Data (entities + counts, collector rows, Sync Now, effects, Backfill) | the foundation's `data-*` pages (`extensions/oxplow-foundation/pages/`) |
| UI: credentials per extension | the foundation's `extensions-credentials` page (`extensions/oxplow-foundation/pages/`) |
| `v_data_entity` (live: models + unsynced entities; `row_count` for the pinned one), `v_effect_backfill` (live: what a backfill would react to), `v_replacement` (live: each replaceable component with its candidates and the chosen one), `v_provider_instance` / `v_provider_credential` / `v_provider_collector` (live: the provider instances, their credentials' presence and sign-in state, their collectors' reads) | `crates/oxplow-app/src/live_models.rs` |

**Decisions (epic tsk289, 2026-09-27).**

- **Storage is a re-syncable cache in the main DB,** not the attached
  per-extension DB the first design sketched. Each entity is a table
  `ext__<extension>__<entity>` (dashes → underscores) plus its
  `v_<extension>_<entity>` view.
  - Oxplow does all the writing from the declared schema, and no
    extension SQL runs on writes, so isolation buys little.
  - One pool and plain views keep `oxplow_query_sql` simple.
  - Every run writes all of a collector's entities in **one transaction**,
    so a failed run changes nothing.
  - A changed column set rebuilds the table.
  - The store refuses to replace a view it doesn't own: a core view, or
    one from another extension. Extension `work` plus entity `item` can't
    shadow `v_work_item`.
  - `drop_extension` removes an extension's tables, views and state.
- **Permission.** An exec collector runs code, so its extension must
  declare `permissions.process` (and the `network` hosts and `secrets` it
  uses); the loader refuses it otherwise.
  - A person approved it when the extension was deployed, and a deployment
    is immutable, so the script that runs is the script that was approved
    (see [architecture.md](./architecture.md) → "A repo's config never runs
    a program without a deployed permission").
  - A teammate who pulls the repo deploys the extension themselves, and a
    changed script needs a new deploy.
  - Running is the `oxplow.collector.sync { collector }` command — from the
    UI, a page action, the scheduler or an agent alike. It never approves
    anything: a deploy that adds a permission asks a person, and an
    agent's becomes a proposal.
- **Environment.** The entry gets `PATH`, `HOME`, its declared `env`
  names, its declared `credentials`, `OXPLOW_EXTENSION_DIR` and
  `OXPLOW_COLLECTOR_ID`. Its stdin is `{ "event": … }` (the event
  envelope) when an event triggered it, `{}` otherwise, written on its
  own thread and closed, so a program that never reads it can't block.
  It runs with a 120 s timeout and a 64 MB stdout cap, and both output
  pipes are drained so it can't deadlock.
- **Network** (tsk324, `net_sandbox.rs`). `network: [api.github.com,
  "*.githubusercontent.com"]` lists the hosts an exec collector may reach:
  - **Pattern syntax.** Lowercase names; `*.` means subdomains only, not
    the bare domain.
  - **Part of the permission.** The hosts must sit under the extension's
    `permissions.network`; widening the list needs the permission, so a
    deploy that adds a host asks.
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
    the listing's `network_enforced` is false, so the deploy prompt says
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
  - Anyone sets them, as the command `oxplow.collector.set_credential`
    (`value` is `writeOnly`: no record, event or result carries it; only
    names some collector of that extension declares).
    Listings carry `credentials: [{name, set}]`, never values.
  - An unset credential is simply not passed; the script decides (the
    GitHub example falls back to `gh`). A keychain error fails the run.
  - A name can't be in both `env` and `credentials`, or be `PATH`, `HOME`
    or `OXPLOW_*`.
  - A secret reaches the script only if its extension declares the name
    in `permissions.secrets` and the person has set the value.
  - `collector_runner::Collectors` bundles root, store, the
    database and event schemas, secrets and the gateway — the context
    every list/run/set call takes (`Collectors::of(svc, root)`).
- **Where it runs.** Collectors run from the **primary** stream's worktree,
  and their data is project-global, like dashboards.
- **Errors.**
  - `RunCollectorError` distinguishes `NotFound`, `Denied` (its extension
    lacks a permission; nothing ran), `Disabled`, `Failed` (it ran; recorded with its
    event, keeping the last good row counts) and `Storage`.
  - A page reading an uncollected entity gets "reads `v_x`, which hasn't
    been collected yet. Run collector `ext/id`…" in place of SQLite's
    bare "no such table".
- **Scheduling.** A background loop (`spawn_scheduler`, started from boot
  on `Services::spawn_every`, so `Services::shutdown` stops it)
  calls `run_due_collectors` once a minute: every collector with
  an `every:` trigger that's due (`due_collectors`, which is pure and
  tested) runs as the `oxplow.collector.sync` command with `Actor::System` — so
  a scheduled run is audited and logs `command.executed` like one from
  the UI (tested: `the_scheduler_runs_collector_sync_through_the_bus`).
  A disabled collector never runs unattended: the command refuses it.
- **Schema.** A synced entity is a model its extension owns: its
  description is its doc plus the joins it documents (`relations`), and
  its contract is its declared columns with their docs, refreshed when the
  declaration changes (`register_entity`). Settings → Data also lists a
  declared entity that hasn't synced (`kind: declared`, no count).

## Change analysis

A **change** is one diff: a `commit` (vs its first parent), an `effort`
(start snapshot → end snapshot, or → the working tree while open — limited
to the effort's own files, `v_effort_file`, since its bracket also holds
what other threads and the person changed), an
agent `turn` (its `start_snapshot_id` → its end `snapshot_id` — what the
turn changed, even across a snapshot taken mid-turn; a turn still
running is `NotFound`; target `{"kind":"turn","turnId":"trn12"}`, P2.10,
V101), a stream's `working` tree (vs HEAD) or a `range`: any two
revisions, each spelled as a `rev` is (`working`, `snap:<id>`,
`git:<rev>`), keyed `<base>..<head>` by the sides as resolved (a git side
by its full sha, so `git:HEAD` and the sha share a row), the base empty
for the whole tree at the head (`..snap:5`); target `{"kind":"range",
"base":"snap:3","head":"working","streamId":"str1"}` (the primary stream
when none). A range is the whole diff (no own-files limit), analyzed as a
commit is; one headed at the working tree moves with the stream and is
kept current with the `working` change (below). V101 rebuilt `change` to
widen its kind CHECK, emptying the cache first (rows are recomputed on
demand); `V105__change_range` widened it again, keeping the rows.
`crates/oxplow-app/src/change_analysis.rs`
analyzes it and stores the rows behind `v_change*`; pages (the
oxplow-bundled change cards) only read them.

- **Getting one.** A query that names it, `SELECT id FROM v_change WHERE kind = 'commit' AND target = :sha` (or `kind = 'working' AND target = ''`, the query's stream; or `kind = 'range' AND target = :start || '..' || :end`, the two sides as params — a `start` left NULL would make the whole value NULL, so a page defaults it to `''`), creates it and starts its analysis ("Made on demand" above): how a declared page gets its `change_id`. `ensure_change(target)` (IPC and MCP; not hinted read-only, since it stores the analysis) returns
  the `v_change` row, computing first if needed. It diffs its two
  revisions (`v_change.base_revision` → `head_revision`) through `Trees`
  (shared with the `diff` IPC, `.context/vcs.md`), reads the
  first 200 changed files' contents, runs `code_analysis::analyze_files`
  (tree-sitter metrics per side, churn, import deltas), and builds rows:
  - files: status, +/−, zone (project zone rules), `is_test` (the
    built-in test-file conventions plus the project's `testing.testPaths`
    globs — `change_analysis::TestPaths`, read when the analysis runs, so
    a change in config applies to the next analysis and earlier stored
    changes keep what they were computed with; a function in such a file is
    a test function). The "look
    here first" score left core (P7.B5, V137 dropped its columns,
    `v_change_file` v2): oxplow-bundled's model `change_interest`
    (`v_oxplow_bundled_change_interest { change_id, path, interest,
    reasons }`) computes it in SQL over `change_file` and
    `change_function` — `(1 + log2(1+lines)) × (1 + 0.6·Σcomplexity↑) ×
    (1 + 0.4·Σparams↑) × (1 + (longest new fn − 60)/40)` — using SQLite's
    math functions (`LIBSQLITE3_FLAGS = -DSQLITE_ENABLE_MATH_FUNCTIONS`
    in `.cargo/config.toml`; every database open probes them and refuses
    a build without, tsk726);
  - functions: added / deleted / modified (signature and/or body), deltas,
    churn and churn share; unchanged ones aren't stored;
  - imports: added/removed with zones, `cross_zone` for new boundary
    crossings;
  - co-change left core (P7.B5, V138 dropped `change_co_change`):
    oxplow-bundled's `co_change_pair` (materialized over the commit
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
  oxplow-foundation's collector's, on every ref move (P7.B5,
  [code-quality.md](./code-quality.md)).
  Closed efforts and turns scan their end snapshot: `Trees` reads any
  revision (P5.B2).
- **Caching.** A change is keyed by (stream, kind, target) — commit shas are
  resolved to full ids, so `HEAD` and a short sha share one row. Commits,
  turns and closed efforts are computed once (an effort that closes since
  is recomputed: its head moved). `oxplow_ensure_change` reads a working tree's
  or an open effort's stored analysis, computing it the first time.
- **Two stages.** Stage one is the file list alone:
  `refresh_files(target)` diffs through `Trees` and stores `change_file`
  (status, +/−, zone, `is_test`) — no file is parsed — stamping
  `v_change.files_at` / `files_events_to`, and for a working tree git's
  `in_progress` operation and `conflicted` count (`v_change` v2, migration
  V2). Stage two is everything else (functions, imports, tests,
  duplicates), stamped `computed_at` / `events_to`. Both write the file
  rows; a list that saw older events than the stored one is dropped
  (`change_store::write_files`). Readers of the file list (the
  Uncommitted panel) read stage one; readers of the deep rows compare
  `events_to` with `files_events_to` to know whether it's caught up.
- **Keeping it current (P7.B4).** The `change.analyze` async pump consumer
  (`change_reactor.rs`) recomputes them as the stream moves: on a
  `snapshot.taken` that recorded files (not `unchanged`) or a
  `vcs.head.moved`, it first runs stage one for the stream's `working`
  change, its `range` changes headed at the working tree
  (`SqliteChangeStore::list_working_ranges`; a range between two fixed
  revisions is immutable, computed once like a commit's) and the open
  efforts' (one without a start snapshot is
  skipped): only the event's thread's effort when a thread's take made
  it, every open effort on the stream otherwise — so the lists are always
  current. `watch_working_files` restates the same working-headed lists
  on a ref or index move. The deep analysis is
  **paced** (`change_reactor::deep_pacing`, the same mechanism as
  collectors' — metrics.md "Pacing"): a take records core's job in
  `pending_run` (`owner` `core`, `id` `change/working/<stream>`,
  `change/effort/<effort>` or `change/range/<stream>/<base>..<head>`) and `crate::pacing` runs `refresh_change`
  once 20 s pass with no further take, at most every 2 minutes; a HEAD move
  or an `agent.turn.ended` (the moment someone looks at a turn's work)
  analyzes at once and clears the job. Each analysis records
  `change.elapsed_ms`; its duplicate scan is timed in
  `v_code_quality_scan` (scope `change <id>`). On `effort.finished` it recomputes that effort's change
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
  on `v_change*`): `useChange` re-ensures when `v_change` changes, so
  asking for a change whose revisions are already stored writes nothing
  (`get_or_create` only reads it) — a write there would announce
  `v_change` and every open change page would re-ask forever (tsk1024). The
  in-memory `ChangeStale` / `ChangeAnalyzed` / `CodeQualityScanned` and the
  invalidation loop are gone. Concurrent requests for the same change
  return it `running`.
- An effort without a start snapshot is an error, not an empty diff.

Deleted / skipped tests and removed assertions are `v_change_function`
(deleted `is_test` rows) plus `v_change_test_file`, read by the
oxplow-bundled Tests Weakened page. Missing co-change is oxplow-bundled's
`v_oxplow_bundled_change_co_change`.

**Still target:** network enforcement off macOS; AI-role columns.
(Extension-declared metrics and dimensions, fact and entity, are current:
see extensions.md.)

## Relation to other docs

- [metrics.md](./metrics.md): the fact substrate this generalizes. Still
  authoritative for facts, captures, the cube and fact collectors.
- [extensions.md](./extensions.md): how pages and extensions consume this
  layer.
- [ai-providers.md](./ai-providers.md): the AI functions sources and pages
  can use.
- [collection.md](./collection.md): test-run and coverage ingest, which
  becomes the tests & coverage source.

## Authority on host SQL reads

Host-only `SqlQuery.model_policy` checks the models SQLite reports during
preparation before stepping the statement. It also traverses `model_input`
`ref` lineage so materialization cannot hide dependencies. A failed policy
restores the connection's authorizer and query-only state. Scope calls and
collector `input` use the same policy through `RunContext`; core reads have
no extension authority. Exact `sql.read:<v_model>` needs are additive; bare
`sql.read` keeps the existing whole-layer meaning. Computed metric table
functions refuse bounded authority until their precomputation has a matching
policy. No extension receives a policy callback or a database connection.

Private extension state does not pass through SQL models or query overlays.
Script hosts can obtain a separate StateSource from the gateway's host-owned
project resources, then read a namespace derived from their RunContext. Bare
gateways have no state source. This does not change `run` or oxplow_query_sql's
semantic read contract.

## Generation reader boundary

`SqlGateway::with_activation` gives the gateway and all of its clones the
project's activation boundary, including metric gateways constructed before
boot (a gateway built alone has its own, never activated). The first
activation is the required-bundled baseline staged at boot. `run`, `check`, `recent_events` and `view_names`
hold a reader pin through their complete operation; nested metric materialization
inherits the task-local pin rather than reacquiring the fair lock. Results report
the pinned generation, not a later filesystem fingerprint. Candidate model
validation stays outside the active reader scope, and a gateway whose boundary has
not activated a generation serves the current load. Other synchronous metadata and native/background entrypoints
still need an audit for complete reader coverage.

Entity collector completion requests coordinated reactivation only when the
registered entity schema differs from the signature captured with the active
generation (view, version, SQL, columns and key). Ordinary data changes keep the
published runtime and provider processes. Until a generation is activated,
model consumers compile against the current fingerprint.
