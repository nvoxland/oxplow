# Metrics — the unified metric substrate

What this doc covers: oxplow's **metric substrate** (epic tsk213) — one durable,
typed model for any deterministically-computable number tracked over time, the
successor to `effort_observation` and (eventually) `code_quality_*`. Coverage,
tests, clippy findings, token usage — all become *metric definitions* over the
same tables, queryable by the agent (MCP) and the renderer (IPC), and surfaced
on the Metrics page.

Status: substrate + read surface are **live** and are now the **sole** store for
coverage/test/analysis facts — the legacy `effort_observation` table was
**dropped** (tsk215). The effort-review panel reconstructs its rows from the
substrate (`CollectionService::effort_observations_from_metrics`); the
`EffortObservation` type survives only as the read/IPC shape.

> **⚠ In-flight successor — the fact substrate (epic tsk12).** Everything below
> from "## The model" down describes the **V38** substrate, which is still the
> live path. A **second, inverted** substrate is being built *alongside* it
> (dual-write phase) and will eventually replace it. Read the next section first;
> the V38 sections stay accurate until the cutover (tsk20). New work targets the
> fact substrate, not V38.

## The fact substrate (epic tsk12 — the inversion, in flight)

**The defect being fixed.** V38 has facts and metrics *inverted*. `metric_sample`
is labelled "the BI fact grain" but holds *pre-aggregated, per-metric* values
(a count-over-threshold, a `tree:.` repo sum) — aggregation baked in by the
collector. `metric_finding` holds the *atomic, re-aggregatable* facts
(function→complexity, test-case→pass/fail, lint hit) — but ephemerally, as
CASCADE-with-run drill-in, not as a durable queryable series. So a new metric
over the same reality needs a *new gauge that re-walks the code*: aggregation is
welded to collection.

**The fix (headless-BI / semantic-layer model).** Invert the source of truth:
- **facts** = durable atomic measurements (the real grain) — the source of truth,
- **measures / dimensions** = the conformed catalogs facts are typed by,
- **metrics** = *aggregation/formula definitions computed over facts at read time*
  — never a second pile of stored rows. The materialized series is a rebuildable
  cache, not the truth.

### Schema — `crates/oxplow-db/migrations/V43__metric_facts.sql`, store `crates/oxplow-db/src/fact_store.rs` (`SqliteFactStore`)

- **`measure`** — the namespaced catalog of *fact types*: `key` (`oxplow.*`
  reserved), `title`, `unit`, `subject_kind` (the grain), `temporal_semantics`
  (`additive` | `semi-additive` | `non-additive` — additivity **over time**:
  tokens additive, complexity/coverage semi-additive, ratios non-additive),
  `component_role` (`numerator`|`denominator`|`none`), `scope`, `description`.
  Seeded built-ins: `oxplow.complexity`, `oxplow.fn_length`,
  `oxplow.parameter_count`, `oxplow.todo`, `oxplow.coverage`, `oxplow.test_case`,
  `oxplow.lint_hit`, `oxplow.duplicate_lines`, `oxplow.tokens`,
  `oxplow.cycle_time`.
- **`dimension`** — the namespaced slice-axis catalog: `key`, `label`,
  `value_type`, `subject_kind`, `vocabulary_json`, `scope`, `promoted` (whether a
  generated column + expression index exists). Seeded: `oxplow.language`,
  `oxplow.severity`, `oxplow.status`, `oxplow.branch`, `oxplow.model`,
  `oxplow.agent`, `oxplow.package`, `oxplow.test_suite`. **Declare-to-collect**
  (planned, tsk17): a fact may only be emitted on defined measures/dimensions;
  historical facts carrying a now-undefined dim are kept but hidden as a slice
  axis (the axis list is catalog-driven).
- **`subject`** — the subject hierarchy (file→package→repo) for roll-ups.
- **`metric_capture`** (the renamed/generalized `metric_run`) — the **one context
  row**: it holds ALL the "when/where/who/trust" metadata so it isn't duplicated
  on every fact. `producer`, `trigger`, `status`/`error`, `scope`; when
  `captured_at`/`ended_at`; where `snapshot_id`/`closest_git_version`/
  `git_version_exact`/`branch`/`basis_ref`; who `stream_id` (NOT NULL, the CASCADE
  scope) / `thread_id` / **`effort_id`** (nullable, `ON DELETE SET NULL` — the
  *producing* effort, stamped only when unambiguous; ledger-backfilled otherwise);
  trust `provenance`/`source`. **Captures are durable** (they carry the facts'
  context — no independent sweep).
- **`fact`** — the durable atomic measurement (folds `metric_sample` +
  `metric_finding`): `capture_id` **NOT NULL** (→ all context via the capture),
  `measure_id`, `value`, `numerator`/`denominator`; subject `subject_kind`/
  `subject_ref`/`path`/`line` (location-at-capture); reported finding metadata
  `severity`/`rule`/`detail` (null for pure measurements); `dims_json` (long-tail
  dims). **No when/where/who columns** — those are the capture's.
- **`metric_spec`** (`V44__metric_spec.sql`, tsk29) — the **metric-as-a-spec**
  catalog (the third catalog beside measure + dimension). A metric is NOT a stored
  sample stream (V38's `metric_definition` *owned* `metric_sample` rows); it is a
  **spec computed over facts at read time**: `key` (`oxplow.*` reserved), `title`,
  `unit`, `source_measure` (the measure whose facts it aggregates; NULL for a
  formula metric), `aggregation` (`count`|`count_distinct`|`sum`|`avg`|`min`|`max`|
  `last`|`p95`|`ratio` — how source facts combine *within* a capture; cross-time
  collapse is the source measure's `temporal_semantics`, not stored here),
  `filter_json` (the conjunctive predicate that turns a raw measure into a
  count-over-threshold), `formula` (derived-metric spec referencing other metric
  keys; NULL for a base), `sliceable_dims_json`, presentation
  `direction`/`target`/`warn_at`/`fail_at`/`display_kind` (`gauge`|`findings`|
  `test`|`coverage`|`event` — read-time only; severity/threshold-state are DERIVED
  from `value` × these, never stored on a fact), `scope`/`category`/`language`.
  **Additive** beside the old `metric_definition` (still FK-referenced by the V38
  `metric_sample`/`metric_finding`); the retire migration (tsk20) drops the V38
  cluster once reads flip (tsk26). No seed rows — specs seed from resolved config
  with the read-flip.

`SqliteFactStore` API: `upsert_measure`/`get_measure`/`list_measures`,
`upsert_dimension`/`list_dimensions`, `upsert_spec`/`get_spec`/`list_specs`,
`record_capture`, `record_facts(capture, facts)` (atomic — inserts the capture,
backfills `capture_id` into every fact, commits together), `get_capture`,
`facts_for_measure` (joined to the capture for the time/version/effort spine),
`facts_for_captures` (the attribution-by-claim read), `aggregate_ratio`.

### The aggregation engine — `crates/oxplow-app/src/metric_engine.rs`

`MetricEngine { facts: SqliteFactStore }` turns facts into metrics at read time:
- `Aggregation` (`Count|Sum|Avg|Min|Max|Last|Ratio`, `parse`),
  `Temporal` (`Additive|SemiAdditive|NonAdditive`), `FactFilter`
  (`min_value` / `severity` / `dim_eq` — the count-over-threshold + slice filters).
- Pure cores: `aggregate_series(facts, agg, filter, group_by)` → one `SeriesPoint`
  per capture (preserving time order), optionally one series per group-by
  dimension value; `range_value(series, temporal)` collapses a series to one
  number the additivity-correct way; `compute_rollup(facts, dimension)` →
  `RollupRow`s (latest-per-subject, summed per dim value, largest first). Ratio
  re-aggregation is Σnumerator/Σdenominator via `numerator`/`denominator`, never a
  naive average of percentages. `dim_value` reads the `severity`/`rule` columns
  and `package`-from-path directly, else `dims_json[key]`.
- Async wrappers `MetricEngine::series(measure_key, agg, filter, group_by)` and
  `rollup(measure_key, dimension)` fetch a measure's facts and aggregate.
- **Spec-driven reads** (tsk29 — a metric *key* → its computed result): given a
  `MetricSpec`, `series_for_spec(spec, group_by)` / `rollup_for_spec(spec, dim)` /
  `headline_for_spec(spec)` resolve the spec's `source_measure` + `aggregation`
  (`FactFilter::from_json` parses `filter_json`) and run the pure cores;
  `headline_for_spec` collapses across time per the *source measure's*
  `temporal_semantics`. A formula metric (no `source_measure`) yields empty/None;
  an aggregation the engine can't yet compute (`count_distinct`/`p95`) or a
  malformed `filter_json` is a surfaced `DomainError::Invalid`, never a silent
  wrong number. This is the bridge the read flip (tsk26) and UI (tsk18) consume.

### Producers — dual-writing facts beside the legacy samples

Each producer writes atomic facts through `record_facts` (a lightweight capture +
the facts), **additively** beside its existing V38 sample/finding write, so the
tree stays green through the migration. Landed:

| producer | where | facts |
|---|---|---|
| tokens | `token_usage.rs` | one `oxplow.tokens` fact per model (`oxplow.model` dim), capture per Stop |
| effort lifecycle | `task_service.rs::project_effort_lifecycle_metrics` | one `oxplow.cycle_time` fact per close, subject=effort; capture **stamps `effort_id`** (unambiguous — this producer knows the exact effort) |
| lint hits | `collection.rs::mirror_analysis_metrics` | one `oxplow.lint_hit` fact per finding (severity/rule/detail columns + file location) |
| coverage | `collection.rs::observe_coverage` | one `oxplow.coverage` fact per file (value=line-%, num/den=covered/instrumented → engine re-derives Σcov/Σinstr) |
| test cases | `collection.rs::record_test_run` | one `oxplow.test_case` fact per case, status as the `oxplow.status` dim (+ `oxplow.test_suite`) |
| duplication | `oxplow-rpc/…/code_quality.rs::run_duplication_scan_at` | one `oxplow.duplicate_lines` fact per duplicate block (value=line count, subject=`path:start-end`, peer side in `detail`); capture stamped with the **primary stream** (a scan has no natural stream) + tree `basis_ref` |

Wired into `Services` as `fact_store: Arc<SqliteFactStore>` +
`metric_engine: MetricEngine`; `TaskService`/`CollectionService`/
`TokenUsageService` carry the fact store; the duplication write lives in the rpc
layer (`svc.fact_store`). Still to come (see the epic's tasks): the **code-gauge
unbake** (per-function complexity/length/param + marker facts; count-over-threshold
becomes a metric spec — the design-heavy keystone).

**Decision — nudges stay OUT of the substrate (tsk24).** The advisory nudges
`collection.rs` fires (report-less, coverage-target, commit-hygiene,
gauge-crossing) are *operational events* — reactions to state × policy, not
measurements of code state — and are already durable in `agent_nudge`. Modeling
them as facts (an `oxplow.nudge` event measure + run-less captures) would dilute
"facts about the code" for marginal telemetry value, so they are intentionally
**not** dual-written. Revisit only if nudge-rate-over-time analytics become a
real ask.

### Read surface (MCP, additive)

`crates/oxplow-mcp/src/lib.rs`, agent-only in the surface-parity MANIFEST, beside
the untouched V38 sample/definition tools:
- `list_measures` / `list_dimensions` — the two catalogs (optional scope/
  subject_kind filter).
- `list_facts(measure_key, limit)` — raw atomic facts, most-recent, with the
  capture spine.
- `metric_series(measure_key, aggregation, group_by?, min_value?, severity?)` —
  the metrics-as-definitions read: one aggregated point per capture, optionally
  sliced by a dimension.
- `metric_rollup(measure_key, dimension?)` — the by-dimension breakdown.

### Catalog authoring surface (`measures:` / `dimensions:` config — workstream E)

Custom fact types and slice axes are **pluggable**, namespaced exactly like
metrics (`oxplow.*` reserved — those are the migration seed; config only *adds*
global/project entries). `crates/oxplow-config/src/lib.rs`:

```yaml
measures:                          # custom fact TYPES a collector may emit
  - key: acme.api_latency
    unit: ms
    subjectKind: endpoint
    temporalSemantics: non-additive   # additive | semi-additive | non-additive
    componentRole: numerator          # none | numerator | denominator
dimensions:                        # custom conformed slice axes
  - key: acme.license
    valueType: categorical            # categorical | numeric | temporal | entity-ref
    vocabulary: [MIT, Apache-2.0]     # optional controlled value set
    promote: true                     # request a generated column + index (see below)
```

- **Definition-only** (no `use:`/`key:` split like metrics — you declare a fact
  type / axis, you don't "enable" one). `validate_measures`/`validate_dimensions`
  mirror `validate_metrics` (namespacing, `oxplow.*` reserved, per-key
  uniqueness, enum checks). `resolve_measures`/`resolve_dimensions` merge the
  **global + project** scopes (project > global) into flat `Resolved*`.
- **Global scope** = `<global_config_dir>/{measures,dimensions}/*.yaml`
  (`load_global_measure_entries`/`load_global_dimension_entries`); auto-active in
  every project (unlike a global metric, which needs a project `use:`).
- **Boot seeding:** `MetricsService::seed_catalog()` runs once at boot and on
  every `ConfigChanged` (beside `seed_definitions`), upserting resolved
  measures/dimensions into the `measure`/`dimension` tables. `MetricsService`
  holds a `fact_store` via `.with_fact_store()`.
- **Scaffolds:** `MetricsService::scaffold_measure` / `scaffold_dimension` —
  one-call "create a custom measure/dimension" (append config entry or write a
  shareable `<global>/…/<slug>.yaml`, reseed, return the key). The IPC/UI "New
  measure/dimension" buttons that surface these land with the UI task.
- **`promote`** is threaded through but currently **inert** (see tsk28): the
  engine loads all facts and filters in-app, so an index bites nothing until
  reads go DB-side. Carried, not acted on.

**Not yet done:** flipping the *existing* reads (`list_metric_samples`,
`metric_breakdown`, `get_metric_summary`, `list_metric_findings`) onto the engine,
the IPC/Tauri counterparts + TS bindings, the UI pages (incl. a **Dimensions
catalog** page), the metric-spec model (`metric_definition` → `source_measure` +
`aggregation`), **declare-to-collect enforcement** (a gauge emitting an undefined
measure/dim is a run error — lands with the code-gauge unbake), `promote_dimension`
teeth, and retiring V38. Those are the open children of the epic.

---


## Why it exists

`effort_observation` (see [collection.md](./collection.md)) was the first cut,
but it was **effort-scoped, CASCADE-deleted with its effort, and pruned to the
last 10** — so it couldn't answer "how did coverage move over the last month" or
"compare this branch to main." A coverage % or token count is a *datum about the
code/process at a point in time*, not a child of an effort. The substrate fixes
that: **time-primary, durable, dimension-sliceable**.

## The model (typed kinds, one fact table)

Don't force every measure into one blob row — the codebase's existing pattern is
*typed kinds with a uniform mechanism* (`CollectorKind`), and this extends it. A
small set of kinds (`gauge | findings | test | coverage | event`) share a common
envelope; each **projects ≥1 scalar sample** into the one narrow fact table the
explorer/feedback read; rich per-kind structure lives in typed detail.

Schema — `crates/oxplow-db/migrations/V38__metrics.sql`,
store `crates/oxplow-db/src/metric_store.rs` (`SqliteMetricStore`):

- **`metric_definition`** — the catalog. `key` (namespaced; `oxplow.*`
  reserved), `kind`, `title`/`unit`, `direction` (higher/lower/neutral),
  `default_agg`, `grain`, `basis`, `producer`, `description`/`category`/
  `language`, `scope` (built-in|global|project), `dimensions_json`,
  `target`/`warn_at`/`fail_at`. Upserted by `key`.
- **`metric_dimension`** — conformed-dimension catalog (seeded: time, stream,
  thread, effort, git_version, branch, subject, model, agent, language,
  severity, status). Shared meaning across metrics → cross-metric drill-across.
- **`metric_subject`** — subject hierarchy (file→module→package→repo) for
  roll-ups. Declared, not yet exercised.
- **`metric_run`** — a compute event (generalizes `code_quality_scan`): producer,
  status, trigger, provenance/source, snapshot/git/**branch**. One run can feed
  many metrics. Raw events have **no run** (`run_id` NULL).
- **`metric_sample`** — the durable scalar fact (the BI grain). `value`
  (+ `numerator`/`denominator` for ratios so roll-ups RE-AGGREGATE correctly),
  `captured_at` (the spine) + `closest_git_version` + `branch`, optional
  `subject_kind`/`subject_ref`/`path`/`line`, `dims_json`, `provenance`/`source`.
- **`metric_finding`** — located detail for the `findings` kind (generalizes
  `code_quality_finding`): path/line, kind, severity, rule, message, value.

### Time-primary, effort-as-overlay (the key invariant)

A sample carries **NO `effort_id` FK**. It's anchored by `captured_at` +
`closest_git_version`. Efforts (and later commits/releases) are **time-range
overlays** read from `task_effort` (`started_at`/`ended_at`) — so:
- efforts can be garbage-collected without touching a single sample,
- a sample can fall in zero or many efforts,
- a `diff-vs-effort-start` metric stays interpretable via its `basis_ref`
  baseline version after its effort is gone.

"Group by effort" = bucket samples whose `captured_at` ∈ the effort's window
(`SqliteMetricStore::samples_for_effort`). No count-prune, no CASCADE; optional
age sweep only.

### Per-file attribution grain (concurrency-safe efforts, tsk250)

Time-window bucketing alone mis-attributes under **overlapping efforts** (two
threads/streams working at once). So the code gauges emit **two grains**:

- the **headline** `tree:.` sample (one per run) — the repo total every existing
  read uses (Metrics page, trend, `get_metric_summary`, `effort_metric_context`);
- sparse **`file:<path>`** samples (nonzero files only) — the *attribution*
  grain, read **only** by the effort roll-up.

`list_samples` / `samples_for_effort` filter out `subject_kind='file'` so the
headline series is unchanged; `file_samples_for_paths(metric_id, stream_id,
paths)` reads the per-file grain. `CollectionService::effort_metric_deltas`
attributes **per metric family** (each already has a sound key — using time alone
was the bug):

| family | attribution key |
|---|---|
| per-file gauges (`gauge`/`tree`, category `custom`) | the effort's **claimed files** (`task_effort_file`) + stream: Δ = Σ over claimed paths of `(current_file − baseline_file)`, run-relative (a claimed file absent from a run = 0, so a drop-to-zero is seen). NB: analysis is *also* `gauge`/`tree` but is run-attributed, not per-file (it emits no `file:` samples) — the classifier (below) tests the run-kind families **first**, so analysis never reaches this branch (tsk272) |
| operational (`agent.*`/`effort.*`/`task.*`) | the effort's **thread** + window (`sum` flows summed, else before→after) |
| tests + analysis (the unified `"run"` kind) | the effort's **claimed run rows** in the `effort_attribution` ledger (`run:<id>` → `samples_for_runs`), NOT a time window — observe-always, so the ledger claim is the only safe attribution under concurrency (`run_attributed_delta`). Category ∈ {`testing`, `static-quality`} |

The family is chosen by **one classifier** — `classify_effort_attribution(def) →
EffortAttributionFamily` (`crates/oxplow-app/src/attribution.rs`, beside the
write-side `AttributionKind` each family maps to: File↔`FileKind`,
Coverage/Run↔`RunKind`, Window↔no-claim). `effort_metric_deltas` `match`es on it;
adding a new fact-kind is one variant + one match arm, not a scattered if/else
chain (tsk274).
| coverage | observe-always (tsk270): the **absolute** whole-report % is stored per run (`oxplow.coverage.abs_pct`) + per-file instrumented/covered line-sets in the `coverage-detail` finding; the effort-relative **diff-coverage** is DERIVED at read (`diff_coverage_for_effort`) against the claiming effort's start snapshot — `coverage_delta` reads the claimed runs, never a time window |

A gauge with no per-file samples (or an effort with no claims) falls back to the
in-window headline before→after.

### Run attribution grain — the ledger, not the clock (tsk260/tsk269)

Agent-work runs — **tests and analysis** today, **coverage** in Phase 2 — are
**observe-always**: every run writes its `metric_run`/`metric_sample` regardless
of how many efforts are open, attributed through the `effort_attribution` ledger,
never by time window — because parallel sub-agents in one thread run different
runs concurrently and the clock can't tell them apart. All three stamp
`trigger='on-report'`, so one OBSERVE (`runs_in_window_by_trigger`) + one ledger
kind `"run"` covers them. At record time `auto_attribute_run` writes a `claimed`
ledger row for `run:<id>` when the caller named a `task_id` (exact) or exactly
one effort is open; the concurrent case is left for the agent to claim at close
(`claim_runs` on `complete_task`/`update_task`/`amend_effort`). Effort reads
therefore join through the ledger: `effort_observations_from_metrics` and
`run_attributed_delta` list the effort's `claimed` run refs, parse the ids, and
read `samples_for_runs(metric_id, run_ids)` — `samples_for_runs` filters to the
metric, so the test/analysis specs share one claimed set. An effort shows exactly
the runs it owns; a concurrent/unclaimed run can't pollute its rollup. **Coverage**
is effort-relative (diff vs the effort's start snapshot), so it observes the
ABSOLUTE report always and DERIVES the effort diff at read
(`diff_coverage_for_effort`, `coverage_delta`) — a run claimed after close still
yields a diff (tsk270). The mechanic + trait (`AttributionKind`/`RunKind`) live in
`.context/agent-model.md` + `.context/data-model.md`.

### Additivity

Ratio metrics (coverage %, pass rate) store `numerator`+`denominator`. Roll-ups
MUST re-aggregate from components (`aggregate_ratio` = Σnum/Σden), never naive-
AVG a percentage. Non-ratio gauges use `default_agg`.

## Branch tracking

Runs and samples record the **branch** they were captured on (`branch` column,
a conformed dimension), when applicable (NULL for detached HEAD / non-git /
operational metrics). Captured via `oxplow_git::detect_current_branch` in the
code-fact producers; operational producers (tokens) leave it NULL.

## Producers (how samples get written)

Producers are the only thing that writes samples. They're best-effort
side-bands on the host path (a metric write error is logged via `tracing::warn!`,
never fails the host path). For coverage/tests/analysis the substrate is now the
**sole** store (the legacy `effort_observation` table was dropped, tsk215) — the
mirror helpers also write a verbatim `*-detail` `metric_finding` (test
suite/case tree, coverage per-file uncovered lines, analysis payload) so the
panel can reconstruct full detail via `effort_observations_from_metrics`:

| producer | where | emits |
|---|---|---|
| coverage / tests / analysis | `crates/oxplow-app/src/collection.rs` (`mirror_coverage_metric` / `mirror_test_metrics` / `mirror_analysis_metrics`, called from `observe_coverage`/`record_test_run`/`record_static_analysis`) | `oxplow.coverage.abs_pct` (absolute; diff derived at read); `oxplow.tests.{passed,failed,total}`; `oxplow.analysis.{errors,warnings}` + a finding per lint hit + a `*-detail` finding carrying the verbatim payload |
| token-parse | `crates/oxplow-app/src/token_usage.rs` (`project_token_metrics`, called from `on_stop`) | per-model `agent.tokens.{input,output,total}`, `agent.turns`. Tokens only — no derived USD cost (rates move; a stale price table is worse than none) |
| effort-lifecycle | `crates/oxplow-app/src/task_service.rs` (`project_effort_lifecycle_metrics`, called when `update()` closes an effort on an `in_progress` exit) | derived `effort.cycle_time_ms` (close − start, subject=effort) + `task.efforts` (efforts-so-far, the redo-rate signal) from `task_effort`; branch captured when the stream has a worktree |
| nudges | `crates/oxplow-app/src/collection.rs` (`project_nudge_metric`, called from `persist_nudge` after a fired nudge records) | `agent.nudges.fired` (event kind, run-less; value 1, subject=the nudge `kind`) — an agent-activity signal |
| config gauges | `crates/oxplow-app/src/metrics_service.rs` (`MetricsService`) — the author-able runner. Seeds a `metric_definition` per resolved `metrics:` entry; runs each `gauge` on its trigger (`on-snapshot` via the snapshot-batch event in `run()`; `on-effort-complete` via the `task_service.rs` ride-along; `manual` via `run_metric_by_key`) | whatever the project/global `metrics:` entry declares — a `metric_sample` per `MetricReport.sample`, version/branch/snapshot-stamped, subject/dims from the script. The bundled code gauges emit both the headline `tree:.` total and sparse `file:<path>` per-file samples (the attribution grain — see the per-file section above) |

> Navigation / activity (`page_visit`, `usage_event`) are **deliberately not
> projected** into the substrate: they're oxplow-usage telemetry (UI metadata),
> not code or agent-activity metrics, so they stay in their own tables.

Each producer: `upsert_definition` (idempotent) → `record_run` → `record_sample`(s)
→ emit `OxplowEvent::MetricSamplesChanged { stream_id }`.

> **The plan is for these to become bundled plugins** (jaq/Starlark/exec,
> registered via `with_builtins()`) so producers are *content*, not hardcoded
> Rust (tsk218). The hardcoded mirror helpers are the interim. The legacy
> `effort_observation` path has already been **dropped** (tsk215) — the substrate
> is the sole store.

> **Producer-metric registry (tsk286/tsk287).** Because these producers only
> `upsert` their definition at *record* time, the Catalog (a registry of
> *available* metrics) can't discover them before first data. So the canonical
> always-on producer metrics live in **`producer_metrics.rs`**
> (`builtin_producer_metrics()` + `ProducerMetric::definition()`) as the **single
> source of truth**: the producers build their `NewMetricDefinition` via
> `producer_metric(key).definition()` (no inline descriptors), and `catalog()`
> unions the same list. Add/rename a producer metric in **one** place. Coverage's
> red/green thresholds (`target`/`warn_at`/`fail_at`) are policy applied by the
> coverage producer on top of the registry descriptor, not part of the registry.

## Read surface

- **MCP** (`crates/oxplow-mcp/src/lib.rs`): reads `list_metric_definitions`
  (optional language/scope filter), `list_metric_samples` (by key, newest-first),
  `list_metric_findings` (by run id — findings-kind drill-in),
  `get_metric_summary` (latest value + delta-vs-target), and `metric_breakdown`
  (tsk327/330 — rolls a per-file metric up by a **dimension** via
  `SqliteMetricStore::dimension_rollup_for_metric`: the latest
  `subject_kind='file'` sample per file, summed by `dimension` key, largest
  first — the dormant `metric_subject` package grain made concrete; "which
  package / language holds the most complexity/TODOs"). `dimension` is
  `"package"` (default — parent dir) or any per-file `dims_json` key (e.g.
  `language`, tsk319); `stream` is optional (omit ⇒ all streams), matching the
  UI. The same store method backs the IPC
  `metric_dimension_rollup` + the Metric Detail **Breakdown** card (tsk328/319). Authoring/trigger:
  `run_metric` (run a configured gauge now — the `manual` trigger → `MetricsService::run_metric_by_key`)
  and `record_metric` (an **asserted**, run-less sample for CI/agent-reported
  numbers). These four are **agent-only** (classified in the surface-parity
  manifest); the renderer drives compute via config + the runner, not ad-hoc IPC.
- **IPC** (`crates/oxplow-rpc/src/commands/metrics.rs` cores +
  `crates/oxplow-tauri-ipc/src/commands/metrics.rs` Tauri adapters, registered
  in `collect_commands!` + the remote `rpc_dispatch!`): `list_metric_definitions`
  / `list_metric_samples` / `list_metric_findings` (the per-run drill-in,
  `both`-scoped — tsk232) exposed to the renderer with generated TS bindings
  (`MetricDefinition`/`MetricSample`/`MetricFinding`). `list_effort_metric_deltas`
  (tsk250, `ui`-scoped — `commands/effort.rs`) returns the family-attributed
  per-effort roll-up (`EffortMetricDelta`) for the task-page panel; the agent
  gets the same numbers as prompt text via `effort_metric_context`.
- **Event**: `OxplowEvent::MetricSamplesChanged { stream_id }` (coarse — the
  renderer refetches).

## UI

The metric UI is **four separate pages**, each with one job, all reading the one
fact table — no per-metric UI code. Each is registered like the `usage` index
page (`tabState.PageKind`, `pageRefs.indexRef`, `RailHud/sections.ts`, `App.tsx`,
`pageKinds.tsx`) and cross-links to the others in its header. The split happened
in two steps: the Catalog off first (tsk282), then Explorer/Recorded/Detail
(tsk283). The three **observe** pages are read-only; the Catalog is the only one
that **writes**.

- **Metrics Explorer** (`MetricsExplorerPage.tsx` wrapping `MetricsExplorer.tsx`)
  — the marquee page and the rail "Metrics" entry (`indexRef("metrics")` /
  `metricsExplorerRef()`). Header links: "Recorded metrics →", "Configure
  metrics →". A measure's title navigates to the metric's **detail page** (via
  `onOpenDetail` → `metricRef`).
- **Recorded Metrics** (`RecordedMetricsPage.tsx`, `PageKind`
  `"metrics-recorded"` / `recordedMetricsRef()`) — the seeded definitions with
  latest value, trend sparkline, capture branch, sample count; colored by
  `statusColor` (target/`fail_at`/direction). Each `<tr>` adopts browser-style
  click via `useRouteDispatch(metricRef(key))` (plain-click → detail in-tab,
  modifier/middle/right → new tab). Header links: "Explorer →", "Configure
  metrics →". Live-refreshes on `metricSamplesChanged`.
- **Metric Detail** (`MetricDetailPage.tsx` wrapping `MetricDetail.tsx`,
  `PageKind` `"metric-detail"`, routed by `metricRef(key, effort)`) — its own
  page (tsk283), navigated into from the Explorer, Recorded Metrics, and the
  task-page EffortMetrics drill-in (so there's no inline overlay). Back goes
  through `PageNavigationContext` (`goBack`, falling back to Recorded Metrics).
  The metric name is the H1; the definition's **`description`** renders as intro
  text under it (tsk309). Layout is the details layout: a right rail holds the
  range/chart-mode/branch controls + the agg-aware in-range stat, the main column
  the trend chart → paginated recordings table → kind drill-in. See the
  `MetricDetail` component bullet below for what each kind renders.

> **Definition descriptions (tsk309).** Every metric carries a one-line
> `description` (on `metric_definition`). It's inherent to the definition — set
> once and not overridable by a `use:` entry (`resolve_one` reads `def.description`,
> like trigger). Sources: the built-in code gauges (`BuiltinMetric.description`),
> the always-on producers (`ProducerMetric.description` in `producer_metrics.rs`),
> and config `key:` entries (`MetricEntry.description` → `ResolvedMetric` →
> `metric_definition()`).

`MetricsExplorer.tsx` itself (the chart component, P4) is a multi-measure
overlay on one time
  axis + group-by a conformed dimension (`branch`/`subject`/declared dims like
  `model`/`language`) + **line / bar / scatter** viz + target band + legend.
  Inline SVG (no charting lib, like the codebase's other visuals); pure grouping
  in `buildExplorerSeries`, pure pairing in `buildScatterPoints` (two measures ×
  a shared group → one point per group value, e.g. coverage × complexity by
  module). **Saved views** (`metricsPresets.ts`, localStorage): name the current
  measures/group-by/viz and reload it later; the picker also offers **built-in
  presets** (`BUILTIN_PRESETS`: "Tokens by model", "Coverage", "Tests pass/fail")
  and the page accepts an `initialPreset` deep-link. **Effort bands** (tsk233):
  the efforts overlapping the charted window (`list_efforts_in_window`) render as
  faint bands behind the series — hover names the effort, click scopes the chart
  to that window (Clear resets). A measure's title links to its per-kind
  **detail** (via `onOpenDetail`).

- **Metric Recording** (`MetricRecordingPage.tsx`, `PageKind` `"metric-recording"`,
  `metricRecordingRef(runId, {metricKey,capturedAt,value})`, tsk313) — drill-in
  from a **single recording**. The Recordings-table rows on the Metric Detail
  page are clickable when the sample has a `run_id`; clicking opens this page,
  which lists the run's **`metric_finding`s** (`list_metric_findings(runId)`) —
  the located items the gauge counted (file:line · name · value). This is how a
  "count of X" gauge (high-complexity functions, long functions) becomes
  drillable: the gauge **emits findings** alongside its samples (see the gauge
  findings channel below). Degrades to an empty state when a run has no findings.

`MetricDetail.tsx` (+ pure `metricDetailData.ts`, tsk232) is the renderer
`MetricDetailPage` mounts: one view selected from `metric_definition.kind`.
Every kind shows the value trend (+ Δ-vs-first, branch, trust badge); each adds
its drill-in from the latest run's findings (`list_metric_findings`):
**findings** → a findings table, **test** → the suite/case tree (from the
`test-detail` payload), **coverage** → per-file uncovered changed lines
(`coverage-detail`), **event** → top-N subjects, **gauge** → trend only.

The **configure** page (tsk282):

- **Metrics Catalog** (`MetricsCatalogPage.tsx` wrapping `MetricsCatalog.tsx`,
  P4) — a dedicated top-level page (`PageKind` `"metrics-catalog"`,
  `metricsCatalogRef()`, launcher Activity category), the only metrics surface
  that **writes**.
  It's a **registry of everything available**, NOT a list of metrics with
  recorded data — every metric the system can produce is listed via
  `list_metric_catalog`, **grouped by category** (Code gauges / Tests / Coverage
  / Static analysis / Operational), even before any sample exists. `catalog()`
  unions **four** sources, deduped by key: (1) the bundled code gauges
  (`builtin_metrics()`, toggleable); (2) project/global `metrics:` entries
  (toggleable); (3) the built-in always-on producers
  (`builtin_producer_metrics()` — tokens, tests, coverage, analysis, effort
  lifecycle, nudges — listed regardless of recorded data so the user sees they
  exist, tsk286); (4) every other seeded `metric_definition` — installed plugin
  metrics and legacy rows. Each entry carries `toggleable` + `category`: **only toggleable
  metrics** (the code gauges + config entries) show the enable/disable checkbox
  and the target editor; always-on producers/plugins render an "Always
  on" badge and read-only fields (they're free side-bands, not opt-in compute —
  the old "built-in vs hardcoded" split was an artifact; both are `scope:
  built-in` in the DB). Enable/disable via `set_metric_enabled` (→ writes a
  `use:` into `.oxplow/project.yaml`); **inline-edit the target** (tsk233) via
  `set_metric_override` → `MetricsService::set_metric_override` writes the
  target override onto the `use:` entry. **Trigger is inherent to the
  definition** — *when* a metric is collected is a property of what it measures,
  not a per-project knob — so it's shown **read-only** and never user-pickable;
  `resolve_one` reads it from the definition (like `compute`), a `use:` entry
  can't override it, and `set_metric_override` no longer accepts it (tsk290).
  **"New metric"** (tsk234/tsk235) scaffolds a gauge at **project** or **global**
  scope: `scaffold_metric` → `MetricsService::scaffold_metric` writes a starter
  Starlark stub + a `key:` `metrics:` entry. *Project* writes under
  `oxplow/metrics/<slug>.star` + `.oxplow/project.yaml`, returns the project-relative path,
  and the page opens it (`onOpenPage(fileRef(path))`). *Global* writes the script
  + a `metrics:` manifest under `<global_config_dir>/metrics/` (shared across the
  user's projects, via `oxplow_config::write_global_metrics_file`) **and** adds a
  project `use:` so it's active here (a global `key:` define is library content
  until a project opts in — see `resolve_metrics`); the global path isn't opened
  (it's outside the worktree). The runner resolves each metric's `entryFile`
  against the right base dir (`MetricsService::script_base_dir`: `<global>/metrics`
  for a global-scope metric, else the project dir).

Metrics are also surfaced **organically off the Metrics pages** (tsk250): the
task/effort page renders an `EffortMetricsBlock` (`components/EffortMetrics.tsx`)
under each effort — the metrics whose facts the effort touched, as compact
before→after rows **grouped by type** (`metricGroup`), self-hiding when empty and
live on `metricSamplesChanged`. A row drills into the metric's detail via
`metricRef(key, {effortId,start,end})` → `MetricDetailPage` (`metricKey` /
`effort` props) renders `MetricDetail` with an **"In this effort"**
before→after callout (+ per-file count) above the full trend.

Catalog reads/writes: `list_metric_catalog` + `set_metric_enabled` +
`set_metric_override` (RPC cores in `commands/metrics.rs`, Tauri adapters,
`ui`-scoped in surface-parity), backed by `MetricsService::{catalog,
set_metric_enabled, set_metric_override}` and the `MetricCatalogEntry` type.
**Token Analytics is retired** as a bespoke page (tsk233) — its `token-analytics`
tab now renders the **Metrics Explorer** page with the "Tokens by model" preset.
(Page/Usage
analytics stay bespoke: `page_visit`/`usage_event` are deliberately **not**
projected into the substrate — see the producers note above.)

## Adding a metric (today)

1. Pick a namespaced `key` and a kind.
2. In the relevant producer, `upsert_definition` it and `record_sample` with the
   value (+ components for ratios), subject, and dims.
3. It then appears in MCP/IPC reads and on the Metrics page automatically — no
   UI code per metric.

## Authoring surface (`metrics:` config — P3, tsk217)

A project (or the user-global library) declares metrics in YAML — no Rust per
metric. Parsed/validated/resolved in `crates/oxplow-config/src/lib.rs`
(`MetricEntry` / `MetricComputeConfig` / `resolve_metrics` /
`load_global_metric_entries`); the runner (`MetricsService`, tsk225) seeds a
`metric_definition` per resolved entry and runs it on its `trigger`.

Two entry forms in the top-level `metrics:` block:

```yaml
metrics:
  - key: repo.unsafe_blocks          # DEFINE a new metric (full def + compute)
    kind: gauge
    direction: lower-better
    unit: count
    trigger: on-snapshot             # on-report|on-snapshot|on-effort-complete|manual|continuous
    dimensions: [language]
    compute: { runtime: starlark, entryFile: oxplow/metrics/unsafe.star }
  - use: myglobal.todo_density        # ENABLE a catalog metric (+ overrides)
    target: 5
```

- The **gauge** script returns `{ "samples": [ {value, subject?, dims?} ] }` and
  may call the `files(glob)` / `ast_query(text, language, sexpr)` host builtins
  (see [collection.md](./collection.md)). It may **also** return
  `"findings": [ {path?, line?, message?, value?, subject?, rule?, severity?} ]`
  (`GaugeFinding`, tsk311) — the *located items the metric counted* (e.g. each
  high-complexity function). The runner records them as `metric_finding`s on the
  run (kind `gauge-item`); a recording then drills into them via the Metric
  Recording page. The bundled complexity/length gauges emit one finding per
  offending function (`{path, line, message=name, value=complexity|length}`,
  tsk312).
- **Three scopes**, precedence **project > global > built-in** by key:
  - **built-in** — the bundled catalog
    (`oxplow_collect_plugin::builtin_metrics()`; scripts under
    `crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/`, embedded via
    `include_str!` in `builtin_metrics.rs`). Each authored through the **public**
    surface (`files()`/`ast_query()`) — no privileged Rust path — and verified by
    a golden test over a fixture corpus. A project activates one with
    `metrics: - use: oxplow.<lang>.<name>`; the runner builds the collector from
    the embedded script (`BuiltinMetric::collector()`), never a project-disk
    file. Two families:
    - **Language-agnostic code metrics** (tsk314) — one metric, all languages —
      `oxplow.todos`, `oxplow.fn_count`, `oxplow.high_complexity_fns`,
      `oxplow.long_functions`. Built via the `code_gauge` helper with
      `language: ""`; the scripts (under `plugins/metrics/code/`) sweep the
      `source_files()` reader and call a capability (`code_metrics()` /
      `markers()`), so the per-language knowledge lives in `oxplow-code-metrics`,
      not the metric. See "Language-agnostic capability layer" below.
    - **Language-idiom metrics** (`oxplow.<lang>.*`) — concepts specific to one
      language: **Rust** (`unsafe_blocks`, `unwrap_expect_calls`,
      `panic_macros`), **TypeScript** (`any_usage`, `non_null_assertions`,
      `console_calls`, `ts_ignore`), **Clojure** (`defn_count`), **C#**
      (`empty_catch`, `blocking_async_calls`).

    This repo dogfoods the language-idiom Rust/TS sets + all four unified code
    metrics in its own `.oxplow/project.yaml`. The
    complexity/`code_metrics()`-backed gauges and the C# grammar
    (`tree-sitter-c-sharp` → `Language::CSharp` in `oxplow-code-metrics`) landed in
    tsk229/tsk230.
  - **user-global** — `global_config_dir()/metrics/*.yaml`, shared across
    projects, hot-reloaded by the config watcher.
  - **project** — `.oxplow/project.yaml` + scripts under `oxplow/metrics/`.

  `use:` references a catalog key and layers overrides; `key:` defines a new one.
  `oxplow.*` is reserved for built-ins (a project may `use:` one but not
  `key:`-define under it).
- Validation mirrors the plugin rules: namespaced keys, project-relative
  `entryFile` (no `..`), known runtime/kind/trigger/direction; a `use:` with an
  unknown key resolves to a warning (skipped), not an error.

The in-oxplow agent authors these on request via the **`oxplow-metrics`** skill
+ the **`/oxplow:new-metric`** command (assets in `crates/oxplow-plugin/`,
materialized for Claude/Codex/opencode) — "make a metric that counts TODOs" →
a working `metrics:` entry + script + verification, no oxplow-team involvement.

## Targets & feedback (advise-only, P5/tsk220)

A definition's `target` / `warn_at` / `fail_at` (interpreted via `direction`) are
the single source of red/green: the Metrics page colors from them
(`MetricsPage.statusColor`, three tiers) — no hardcoded UI ramps (the coverage
50/80 ramp is retired; the thresholds live on `oxplow.coverage.diff_pct` as
`target: 80` / `fail_at: 50`, set in `collection.rs::record_coverage_metric`).
See [theming.md](./theming.md).

Feedback is **advisory — oxplow never blocks**. Two paths:

- **Coverage-target nudge** (PostToolUse). When an effort's diff coverage lands
  below target, the ride-along fires a **one-shot** nudge ("coverage X% < 80%
  target — add tests…") via the same `persist_nudge` + `additionalContext` path
  as the report-less / commit-hygiene nudges, deduped per-effort by an in-memory
  `nudged_coverage` set.
- **Effort metric deltas** (UserPromptSubmit, tsk231). `CollectionService::
  effort_metric_context(thread)` builds a "# Metric deltas (this effort)" block —
  for every **code** metric (operational `agent.*`/`effort.*`/`task.*` and `event`
  kinds are skipped) it shows `title: baseline → current (Δ ±N)`. It consumes the
  **shared `effort_metric_deltas` core** (tsk253), so the prompt and the task-page
  panel report the **same** numbers — file-attributed for gauges, so under
  overlapping efforts the agent sees only its own effort's effect, not the repo
  total. The first turn a **gauge crosses** its `warn_at`/`fail_at` (`threshold_state`,
  interpreted via `direction`) the line gets a loud `⚠ crossed fail/warn
  threshold` marker, **one-shot** per `(effort, metric)` via an in-memory
  `nudged_gauge` set — on-snapshot gauges run outside any hook, so the crossing
  can't ride the PostToolUse return; the per-turn prompt context surfaces it
  instead. The control-plane joins this with the session-context block into the
  one UserPromptSubmit `additionalContext`. Returns `None` (adds nothing) when no
  metric moved and no crossing is fresh, so steady-state turns stay quiet.

## Gotchas

- **Provenance is the spine** (carried from collection.md): in-process/parsed →
  `observed`; agent-asserted / exec-tier → `asserted` / `plugin-exec:<name>`. The
  UI must never let an asserted number pass for a measured one.
- **Best-effort writes**: producers swallow metric errors so they never break
  the host (collection ride-along, Stop hook). A missing sample is a logged warn,
  not a failure — check daemon logs if a metric doesn't appear.
- **Raw integer ids at the store layer**; the service/IPC boundary maps to/from
  prefixed domain ids (`str1`, `eff1`). `stream_id` is the hard CASCADE scope;
  `thread_id`/`effort_id` are nullable durable/overlay dimensions.
- **TS bindings + event variants regenerate** via the `export_ts_bindings` test
  in `oxplow-tauri-ipc` (`cargo test -p oxplow-tauri-ipc export_ts_bindings`); CI
  fails on an uncommitted diff.
