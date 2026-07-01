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
  tokens additive; complexity + test/lint SNAPSHOTS semi-additive (a run
  replaces the last — V47/tsk42 fixed test_case/lint_hit from V43's wrong
  `additive`); ratios + mean-across-closes measures (cycle_time, task_effort)
  non-additive),
  `component_role` (`numerator`|`denominator`|`none`), `scope`, `description`.
  Seeded built-ins: `oxplow.complexity`, `oxplow.fn_length`,
  `oxplow.parameter_count`, `oxplow.todo`, `oxplow.coverage`, `oxplow.test_case`,
  `oxplow.lint_hit`, `oxplow.duplicate_lines`, `oxplow.tokens`,
  `oxplow.cycle_time` (V43), plus `oxplow.ast_hit` (V45 — a per-file AST idiom
  occurrence; the per-language gauges emit facts on it, distinguished by the
  `oxplow.rule` dim; see the code-gauge section, tsk30).
- **`dimension`** — the namespaced slice-axis catalog: `key`, `label`,
  `value_type`, `subject_kind`, `vocabulary_json`, `scope`, `promoted` (whether a
  generated column + expression index exists). Seeded: `oxplow.language`,
  `oxplow.severity`, `oxplow.status`, `oxplow.branch`, `oxplow.model`,
  `oxplow.agent`, `oxplow.package`, `oxplow.test_suite` (V43), `oxplow.rule`
  (V45 — the lint/idiom name; the engine reads it off the fact's `rule` column).
  **Declare-to-collect**
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
  cluster once reads flip (tsk26). The migration seeds no rows; the **built-in
  specs** are seeded from Rust in `seed_catalog`: the code-metric specs
  (`oxplow.high_complexity_fns` / `long_functions` / `fn_count` / `todos` — a
  `count` over its measure, thresholds via `min_value`; `builtin_metric_specs`,
  tsk23) and the per-language idiom specs (`oxplow.rust.unsafe_blocks` etc. — a
  `Sum(oxplow.ast_hit)` filtered by `dim_eq(oxplow.rule, …)`; `builtin_ast_specs`,
  tsk30). Config/global spec seeding lands with the read-flip (tsk26).

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
  number the additivity-correct way; `compute_rollup(facts, dimension, temporal)`
  → `RollupRow`s, additivity-aware like `range_value` (tsk41): semi-additive →
  latest-per-subject summed per dim value; additive → EVERY fact counts (tokens
  by model is a running total, not the last turn); non-additive → latest per
  subject then per-group Σnumerator/Σdenominator, never a naive sum/average of
  percentages. `dim_value` reads the `severity`/`rule` columns and
  `package`-from-path directly, else `dims_json[key]`.
- Async wrappers `MetricEngine::series(measure_key, agg, filter, group_by)` and
  `rollup(measure_key, dimension)` fetch a measure's facts and aggregate
  (`rollup` parses the measure's `temporal_semantics`, erroring on a malformed
  value rather than guessing).
- **Spec-driven reads** (tsk29 — a metric *key* → its computed result): given a
  `MetricSpec`, `series_for_spec(spec, group_by)` / `rollup_for_spec(spec, dim)` /
  `headline_for_spec(spec)` resolve the spec's `source_measure` + `aggregation`
  (`FactFilter::from_json` parses `filter_json`) and run the pure cores;
  `headline_for_spec` collapses across time per the *source measure's*
  `temporal_semantics`. A formula metric (no `source_measure`) yields empty/None;
  an aggregation the engine can't yet compute (`count_distinct`/`p95`) or a
  malformed `filter_json` is a surfaced `DomainError::Invalid`, never a silent
  wrong number. This is the bridge the read flip (tsk26) and UI (tsk18) consume.
- **T-C1 plumbing (additive, tsk26 prep):** `SeriesPoint` carries `branch` +
  `provenance` (one capture → one of each, taken from the bucket's spine in
  `aggregate_series`); `findings_for_spec(spec, capture_id?)` projects a spec's
  filtered facts as `FactFinding`s (the offenders drill-in that replaces the baked
  `metric_finding` — severity is the fact's reported severity or, absent one,
  DERIVED via the shared `threshold_state(direction, value, warn_at, fail_at)`,
  lifted here from `collection.rs`); `dim_value` gains branch/subject/model
  **pseudo-dims** (off the capture/fact spine, not `dims_json`) so `group_by` is
  uniform server-side. `SqliteFactStore::captures_for_effort(effort_id)` returns
  an effort's captures (the attribution-by-claim spine for T-D). These stay
  non-`Type` (out of `bindings.ts`) until T-C3 wires the IPC.

### Producers — dual-writing facts beside the legacy samples

Each producer writes atomic facts through `record_facts` (a lightweight capture +
the facts), **additively** beside its existing V38 sample/finding write, so the
tree stays green through the migration. Landed:

| producer | where | facts |
|---|---|---|
| tokens (T-B) | `token_usage.rs` | PER-KIND facts on `oxplow.tokens` (one input + one output per model, sliced by the `oxplow.token_kind` dim) + a turn fact on `oxplow.turn`, capture per Stop. `agent.tokens.total` sums both kinds; input/output specs filter by `token_kind` |
| effort lifecycle (T-B) | `task_service.rs::project_effort_lifecycle_metrics` | one `oxplow.cycle_time` fact per close (subject=effort) + one `oxplow.task_effort` fact (subject=task, the efforts-so-far redo signal); both carry `numerator=value, denominator=1` (the measures are non-additive per V47, so Σn/Σd across time = the MEAN across closes, tsk42); capture **stamps `effort_id`** (unambiguous — this producer knows the exact effort) |
| nudges (T-B) | `collection.rs::project_nudge_metric` | one `oxplow.nudge` event fact per fired nudge (value 1, subject=the nudge kind) — the `agent.nudges.fired` spec is `Sum(oxplow.nudge)` |
| lint hits | `collection.rs::mirror_analysis_metrics` | one `oxplow.lint_hit` fact per finding (severity/rule/detail columns + file location) |
| coverage | `collection.rs::observe_coverage` | one `oxplow.coverage` fact per file (value=line-%, num/den=covered/instrumented → engine re-derives Σcov/Σinstr) |
| test cases | `collection.rs::record_test_run` | one `oxplow.test_case` fact per case, status as the `oxplow.status` dim (+ `oxplow.test_suite`) |
| duplication | `oxplow-rpc/…/code_quality.rs::run_duplication_scan_at` | one `oxplow.duplicate_lines` fact per duplicate block (value=line count, subject=`path:start-end`, peer side in `detail`); capture stamped with the **primary stream** (a scan has no natural stream) + tree `basis_ref` |
| code gauges | `metrics_service.rs::run_one_gauge` → `record_gauge_facts` (tsk23) | the bundled code gauges emit a `facts` channel: one fact **per function** on `oxplow.complexity` (high_complexity_fns) / `oxplow.fn_length` (long_functions) / `oxplow.parameter_count` (fn_count), and one per marker on `oxplow.todo` (todos) — the raw grain, for **every** item, not just the offenders the baked count reports |
| per-language idiom gauges | same path (tsk30) | the ~10 idiom gauges (`oxplow.rust.unsafe_blocks`, `oxplow.ts.any_usage`, `oxplow.csharp.empty_catch`, …) emit one **per-file** `oxplow.ast_hit` fact (value=the file's count, `rule`=the idiom slug); the metric is a `Sum(oxplow.ast_hit)` spec filtered by `dim_eq(oxplow.rule, <slug>)` (`builtin_ast_specs`) |

**Fact-attribution spine — `metric_capture.effort_id` (T-D prep, tsk37).** The
read-side effort attribution (T-D) resolves an effort's facts from *its captures*
(`captures_for_effort`). So the effort-scoped producers stamp `capture.effort_id`
at write time using the **same** resolution the run-ledger auto-claim uses —
`CollectionService::resolve_owning_effort(thread, task)`: a named task is
exact-or-nothing (`find_open_for_task`); an unnamed one claims only the single
open effort (`find_single_open_for_thread`), else stays null (deferred to
reconcile). Stamped by: **tokens/turns** (`token_usage.rs`, the effort resolved
once in `on_stop` and threaded to the capture), **tests / lint-hits / coverage /
nudges** (`collection.rs`), and **effort-lifecycle** (`task_service.rs`, which
knows its exact effort). The **snapshot code-gauge** captures are deliberately
NOT stamped — they're whole-tree scans whose baseline predates the effort, so
T-D's File family attributes them by *claimed files × time window*, not by
`effort_id`. `auto_attribute_run` now composes `resolve_owning_effort` +
`claim_run` (the `run:<id>` ledger write is unchanged) so the run claim and the
capture stamp always agree.

**Code-gauge unbake (tsk23) — the keystone, and the one non-mechanical producer.**
A gauge's `MetricReport` gained a third channel beside `samples` (baked headline)
and `findings` (offenders drill-in): `facts: [GaugeFact { measure, value, subject?,
path?, line?, dims? }]` — measure-bound atomics. `record_gauge_facts` resolves each
fact's `measure` against the catalog (**declare-to-collect**, decision #4: a fact on
an undefined measure is dropped with a `tracing::warn!`, never silently written) and
writes the resolvable facts under one capture — the **only** output now (facts-only,
T-C3b). The count-over-threshold headline is the **spec** (`builtin_metric_specs`),
and the equivalence test `code_gauge_facts_reaggregate_to_the_expected_headline`
pins `engine.headline_for_spec(spec) == the expected gauge total` for every bundled
code metric — the proof the inversion is faithful. (Strict `> N` in the old gauge
equals `min_value = N+1` on the integer complexity/length measures.) The 4 code
scripts are unbaked (no `tree:.`/`file:` samples); the baked write path is gone.

**Per-language idiom gauges (tsk30).** The same pattern extends to the ~10
per-language AST idiom gauges, but they don't have a natural per-subject measure —
so they share **one** generic measure `oxplow.ast_hit` (a per-file idiom
occurrence) and are told apart by the `oxplow.rule` dimension (the idiom slug,
carried on the fact's `rule` column via the new `GaugeFact.rule`). Each gauge emits
one per-file `ast_hit` fact (value=that file's count, `rule`=its slug) beside its
per-file sample; each metric is a `Sum(oxplow.ast_hit)` spec filtered by
`dim_eq(oxplow.rule, <slug>)` (`builtin_ast_specs`, seeded in `seed_catalog`). Idioms
sharing the measure never collide because `rollup_for_spec` applies the rule filter
**before** the per-subject rollup. `per_language_gauge_facts_reaggregate_to_the_baked_headline`
pins each spec's `Sum` to its baked headline. The `<slug>` in the script and the
spec MUST match (the equivalence test catches a drift → spec count 0 ≠ baked).

Wired into `Services` as `fact_store: Arc<SqliteFactStore>` +
`metric_engine: MetricEngine`; `TaskService`/`CollectionService`/
`TokenUsageService` carry the fact store; the duplication write lives in the rpc
layer (`svc.fact_store`). Still to come (see the epic's tasks): the **code-gauge
unbake** (per-function complexity/length/param + marker facts; count-over-threshold
becomes a metric spec — the design-heavy keystone).

**Producer specs (T-B).** Each always-on producer metric now has a `metric_spec`
(`producer_metrics.rs::builtin_producer_specs`, seeded in `seed_catalog` beside
the built-in gauge specs) — the aggregation it *is* over the measure its producer
emits facts on, with conformed dims (not extra measures) distinguishing variants:
token in/out slice `oxplow.tokens` by `oxplow.token_kind`; tests slice
`oxplow.test_case` by `oxplow.status`; analysis filters `oxplow.lint_hit` by
severity; coverage is a `ratio` over `oxplow.coverage`. `producer_spec_shape`
holds the `(source_measure, aggregation, filter)` per key. Equivalence tests pin
each spec's engine headline to the baked total (tokens, tests). New V46 measures:
`oxplow.turn`, `oxplow.task_effort`, `oxplow.nudge` (the producers with no prior
measure home); new dim `oxplow.token_kind`.

**Decision reversed — nudges are now IN the substrate (T-B, was tsk24).** The
earlier call kept the advisory nudges (report-less, coverage-target,
commit-hygiene, gauge-crossing) out of the substrate. T-B reverses it: with the
producer-spec layer in place, adding an `oxplow.nudge` event measure + one fact
per fired nudge is cheap and makes the `agent.nudges.fired` operational metric a
first-class spec like every other. The nudge rows in `agent_nudge` stay the
authoritative store; the fact is the analytics grain.

### Read surface (MCP)

`crates/oxplow-mcp/src/lib.rs`, agent-only in the surface-parity MANIFEST. The
measure-level primitives:
- `list_measures` / `list_dimensions` — the two catalogs (optional scope/
  subject_kind filter).
- `list_facts(measure_key, limit)` — raw atomic facts, most-recent, with the
  capture spine.
- `metric_series(measure_key, aggregation, group_by?, min_value?, severity?)` —
  the metrics-as-definitions read: one aggregated point per capture, optionally
  sliced by a dimension.
- `metric_rollup(measure_key, dimension?)` — the by-dimension breakdown.

**The five metric-KEY reads are flipped onto the engine (T-C2, tsk35)** — they
resolve a `metric_spec` by key (seeded catalog) and compute over its
`source_measure` facts, no longer reading the legacy V38 `metric_sample`/
`metric_finding`/`metric_definition` store:
- `list_metric_definitions` → `fact_store.list_specs()` (the spec catalog; each
  row carries `source_measure` + `aggregation`, not a baked sample stream).
- `list_metric_samples(metric_key, limit)` → `series_for_spec` (newest-first,
  capped) — the metric-key ergonomic wrapper over `metric_series`.
- `metric_breakdown(metric_key, dimension?)` → `rollup_for_spec` (default dim
  `oxplow.package`; the old per-stream `stream` arg is gone — facts aren't
  stream-partitioned at this grain).
- `get_metric_summary(metric_key)` → `headline_for_spec` (series collapsed per
  the measure's temporal semantics) + the latest series point's captured_at/branch.
- `list_metric_findings(metric_key, capture_id?)` → `findings_for_spec` — the
  read-time offenders view (args changed from `run_id`).

**The IPC/Tauri counterparts are flipped too (T-C3a, tsk39)** — the four
`oxplow-rpc` cores + Tauri adapters now return spec/fact types
(`MetricSpec`/`SeriesPoint`/`RollupRow`/`FactFinding`), `bindings.ts` is
regenerated, and every metric frontend consumer moved with them (see the "Reads"
table below). Two measure-level IPC reads (`metric_series`/`metric_rollup`) were
added and their parity rows flipped `agent`→`both`. `SeriesPoint` gained
`git_version` + `source` (one-per-capture, like `branch`/`provenance`) so the
recordings table stays intact. Two frontend logic moves: the Explorer's group-by
is now a server `series_for_spec` `group_by` (each point carries `group`); the
`event`-kind subject breakdown is a server `rollup_for_spec("subject")`. The
`test`/`coverage` drill-ins fold into the uniform per-file/per-case `FactFinding`
table — the bespoke suite-tree / line-heat needed the verbatim legacy
`*-detail` payloads (pass/fail status, uncovered line-sets), which are **not in
facts** (the same scope-guard the coverage attribution took in T-D); the
underlying facts still chart + roll up.

**The baked writes are gone (T-C3b, tsk40)** — `record_baked_run` is deleted;
`run_one_gauge` is facts-only (returns the fact count; `record_gauge_facts`
emits `MetricSamplesChanged`). The 4 `plugins/metrics/code/*.star` scripts are
unbaked (emit only `{facts:[…]}`, no `tree:.`/`file:` samples). Any `samples`/
`findings` a script still returns (the per-language idiom scripts, not yet
unbaked) are computed-but-ignored. The legacy V38 `metric_sample`/`metric_finding`/
`metric_definition`/`metric_run` tables + `metric_store.rs` retire LAST in T-E
(tsk20).

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

**Not yet done:** a **Dimensions catalog** UI page; `promote_dimension` teeth
(tsk28); unbaking the per-language idiom scripts (still emit dead `tree:.`/
`file:` samples); and retiring V38 (tsk20). Those are the open children of the
epic. Already landed: the **MCP** metric-key reads (T-C2), the **IPC + bindings +
frontend** read surface (T-C3a, tsk39), the **baked-write removal + 4-script
unbake** (T-C3b, tsk40), and the **effort-attribution read** (T-D —
`effort_metric_deltas` over specs + facts, coverage kept as a scope-guarded
special case).

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

### Effort attribution — spec-driven, over facts (T-D, tsk36)

`CollectionService::effort_metric_deltas` reads the **spec catalog**
(`list_specs()`) and, per **metric family**, aggregates the effort's own **facts**
(no legacy sample read; the `EffortMetricDelta` DTO shape is preserved, so the IPC
+ `EffortMetrics.tsx` are untouched). Two capture-resolution spines back the four
families:

- **claimed files × time** — code gauges are snapshot scans, so their captures are
  **not** effort-stamped; the File family reads them by claimed path + capture time,
  scoped to the effort's stream. Exception: an `on-effort-complete` gauge run KNOWS
  its producing effort and stamps the capture (tsk43 — `GaugeRunContext.effort_id`),
  so its just-after-close capture still counts as the effort's "current".
- **`metric_capture.effort_id`** — the run/operational producers stamp the owning
  effort at ingest (tsk37, `resolve_owning_effort`), so an effort's run + token +
  nudge facts are exactly `captures_for_effort(effort_id)` → `facts_for_captures`.

| family | how the delta is computed |
|---|---|
| **File** — snapshot-scan gauge (`display_kind` ∈ {`gauge`, `findings`}, a source measure, no formula, non-producer, non-operational — includes the `static-quality` built-in code gauges, whose captures are never effort-stamped; tsk43) | Σ over the effort's **claimed files** (`task_effort_file`) of `(current − baseline)` fact value; facts are scoped to the effort's **stream** (worktree). Baseline capture = latest before the effort start; current = latest at/before the effort end (newest when open; a capture STAMPED with this effort — an on-effort-complete gauge run — also counts). A CLOSED effort with no in-window capture yields no row (never a post-close capture, never a fabricated drop-to-zero). A claimed file absent from a capture = 0 (sparse emission → a drop-to-zero is seen). **No claims, or repo-scalar facts with no path** → the repo-wide before→after fallback. `file_delta_from_facts` |
| **Run** — tests (category `testing`) + the `oxplow.analysis.*` producer pair | before→after (or `sum` flow) over `aggregate_series` of the facts of the effort's OWN captures (`facts_for_captures(measure, captures_for_effort)`). Analysis is classified Run via the producer-key check (its facts arrive on effort-stamped run-ingest captures), so it never reaches the File branch (the tsk272 guard) |
| **Window** — operational (`agent.*`/`effort.*`/`task.*`) + formula/event specs | identical read to Run now that captures carry `effort_id`; kept a distinct family only to document it has no run-claim write side. `effort_stamped_delta` serves both |
| **Coverage** (category `coverage`) | **documented scope-guard special case**: still on the legacy detail payload. `coverage_delta_for_spec` resolves the spec's legacy `MetricDefinition` by key and derives the effort-relative **diff-coverage** at read (`diff_coverage_for_effort`) from the run's stored ABSOLUTE per-file **line-sets** (the `coverage-detail` finding). The coverage FACTS carry num/den counts, not line-sets, so migrating this needs a producer change — deferred |

The family is chosen by **one classifier** — `classify_effort_attribution(spec)
→ EffortAttributionFamily` (`crates/oxplow-app/src/attribution.rs`, beside the
write-side `AttributionKind` each maps to: File↔`FileKind`, Coverage/Run↔`RunKind`,
Window↔no-claim). `effort_metric_deltas` `match`es on it; adding a fact-kind is one
variant + one match arm, not a scattered if/else chain (tsk274). A formula spec (no
source measure) falls through to Window and no-ops.

The ledger-run-claim ∪ (the `capture.effort_id` spine) is the intended end state;
T-D lands on the stamped spine alone (the common auto-attributed case). A run
CLAIMED post-hoc (`claim_runs` at close) whose capture wasn't stamped at ingest is
the deferred backfill (tsk38). The now-orphaned legacy reads
(`file_samples_for_paths`, `samples_for_effort`) are swept in T-E (tsk20);
`samples_for_runs` + `list_findings` stay for the kept coverage path + the
`effort_observations_from_metrics` path.

### Run attribution grain — the ledger, not the clock (tsk260/tsk269)

Agent-work runs — **tests and analysis** today, **coverage** in Phase 2 — are
**observe-always**: every run writes its capture/facts (and, for now, dual-writes
`metric_run`/`metric_sample`) regardless of how many efforts are open, attributed
through the `capture.effort_id` stamp (T-D read) + the `effort_attribution` ledger
(the write/reconcile side), never by time window — because parallel sub-agents in
one thread run different runs concurrently and the clock can't tell them apart. All
stamp `trigger='on-report'`. At record time `auto_attribute_run` resolves the
owning effort — when the caller named a `task_id` (exact) or exactly one effort is
open — stamps `capture.effort_id`, and writes a `claimed` ledger row for `run:<id>`;
the concurrent-unnamed case is left for the agent to claim at close (`claim_runs`
on `complete_task`/`update_task`/`amend_effort`). The `effort_observations_from_metrics`
read still joins the ledger (`run_ids` → `samples_for_runs`); the metric-delta read
(above) joins `capture.effort_id`. **Coverage** is effort-relative (diff vs the
effort's start snapshot), so it observes the ABSOLUTE report always and DERIVES the
effort diff at read (`diff_coverage_for_effort`, `coverage_delta`) — a run claimed
after close still yields a diff (tsk270). The mechanic + trait
(`AttributionKind`/`RunKind`) live in `.context/agent-model.md` +
`.context/data-model.md`.

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
| config gauges | `crates/oxplow-app/src/metrics_service.rs` (`MetricsService`) — the author-able runner. Seeds a `metric_spec` per resolved `metrics:` entry (+ a legacy `metric_definition` until the read-flip); runs each **gauge** (`resolved_gauges()` = config `gauges:` ∪ `use:`-enabled built-ins) on its trigger (`on-snapshot` via the snapshot-batch event in `run()`; `on-effort-complete` via the `task_service.rs` ride-along; `manual` via `run_metric_by_key`) | one `fact` per `GaugeFact` the script emits (bound to a defined measure in the gauge's `emits`), version/branch/snapshot-stamped, under one `metric_capture`. Facts-only (T-C3b): `run_one_gauge` writes nothing but facts; any `samples`/`findings` a script still returns are ignored |

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

> **Flipped (T-C2, tsk35 + T-C3a, tsk39):** the metric-key reads on BOTH the MCP
> and the IPC surface now read the fact substrate via `MetricEngine`
> spec-wrappers, NOT this V38 store. The paragraph below is the historical V38
> shape, kept for context; the current IPC wiring + types are in the **IPC**
> bullet.

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
  `run_metric` (run a configured gauge now — the `manual` trigger → `MetricsService::run_metric_by_key`;
  returns `facts_recorded`) and `record_metric` (an **asserted FACT** on the
  metric's source measure, under a `provenance: asserted` / `source:
  agent-reported` capture — flipped off the legacy sample write in tsk41 so the
  fact-based reads actually see it; a formula spec is rejected). These four are
  **agent-only** (classified in the surface-parity manifest); the renderer
  drives compute via config + the runner, not ad-hoc IPC.
- **IPC** (`crates/oxplow-rpc/src/commands/metrics.rs` cores +
  `crates/oxplow-tauri-ipc/src/commands/metrics.rs` Tauri adapters, registered
  in `collect_commands!` + the remote `rpc_dispatch!`) — **flipped onto specs +
  facts (T-C3a, tsk39)**, mirroring the MCP wiring: `list_metric_definitions` →
  `list_specs` (`MetricSpec`), `list_metric_samples(metric_key, limit, group_by?)`
  → `series_for_spec` (`SeriesPoint`, newest-first; `group_by` slices server-side),
  `metric_dimension_rollup(metric_key, dimension)` → `rollup_for_spec` (`RollupRow`,
  also serves the `event`-kind `subject` breakdown), `list_metric_findings(metric_key,
  capture_id?)` → `findings_for_spec` (`FactFinding`, `both`-scoped — the per-capture
  drill-in, args changed from `run_id`). Two measure-level reads `metric_series` /
  `metric_rollup` are `both`-scoped mirrors of the MCP tools. Bindings regenerate
  to `MetricSpec`/`SeriesPoint`/`RollupRow`/`FactFinding`. `list_effort_metric_deltas`
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
> and config `key:` entries (`MetricEntry.description` → `ResolvedSpec` →
> `spec_definition()`).

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
  **"New metric"** scaffolds the **trio** (measure + gauge + metric) at
  **project** or **global** scope: `scaffold_metric` →
  `MetricsService::scaffold_metric` writes a starter fact-emitting Starlark stub +
  a `measures:` entry (`<key>.count`) + a `gauges:` entry (`<key>`) + a `metrics:`
  spec (`<key>`, `sum` over the measure). *Project* writes the script under
  `oxplow/gauges/<slug>.star` + the three entries in `.oxplow/project.yaml`,
  returns the project-relative path, and the page opens it. *Global* writes the
  script + three manifests under `<global_config_dir>/{gauges,measures,metrics}/`
  (via `write_global_{gauges,measures,metrics}_file`) **and** adds a project `use:`
  so the metric is active here (the global gauge + measure are active
  automatically; a global metric is library content until a project opts in). The
  runner resolves each gauge's `entryFile` against the right base dir
  (`script_base_dir`: `<global>/gauges` for a global-scope gauge, else the project
  dir).

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

## Authoring surface (the four config blocks — epic tsk12, E)

A project (or the user-global library) declares metrics in YAML — no Rust per
metric. The substrate is dimensional, so authoring splits into **four orthogonal
blocks** (matching the real cardinality): `measures:` (fact TYPEs) ← `gauges:`
(fact PRODUCERs) → facts → `metrics:` (read SPECs), sliced by `dimensions:`.
Parsed/validated/resolved in `crates/oxplow-config/src/lib.rs`
(`MetricEntry`→`ResolvedSpec` + `resolve_metrics`; `GaugeEntry`→`ResolvedGauge` +
`resolve_gauges`; `GaugeComputeConfig`; `load_global_{metric,gauge}_entries`); the
runner (`MetricsService`) seeds a `metric_spec` per resolved metric (and a legacy
`metric_definition` until the read-flip, tsk26) and runs each **gauge** on its
`trigger`.

```yaml
measures:                             # the fact TYPE the gauge emits
  - key: repo.todo_count
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive  # additivity OVER TIME
gauges:                               # the PRODUCER (runs a script, emits facts)
  - key: repo.todo
    trigger: on-snapshot              # on-report|on-snapshot|on-effort-complete|manual|continuous
    emits: [repo.todo_count]          # declare-to-collect allow-list
    compute: { runtime: starlark, entryFile: oxplow/gauges/todo.star }
metrics:                              # the read SPEC (the chartable metric)
  - key: repo.todo_count              # DEFINE — a measure aggregation
    sourceMeasure: repo.todo_count
    aggregation: sum                  # count|sum|avg|min|max|last|ratio (within a capture)
    direction: lower-better
    unit: count
    displayKind: gauge                # gauge|findings|test|coverage|event
    filter: { minValue: 1 }           # optional predicate before aggregating
    sliceableDims: [language]
  - use: myglobal.todo_density        # ENABLE a catalog metric (+ threshold overrides)
    target: 5
```

- A **metric** no longer computes anything — it's a pure spec (`sourceMeasure` +
  `aggregation` + optional `filter`, OR a `formula: {op,left,right}` over other
  metrics). Two-axis aggregation: `aggregation` combines facts *within a capture*;
  the source measure's `temporalSemantics` governs the cross-time collapse. A
  `use:` may only re-target thresholds; the structural fields are inherent.
- The **gauge** script returns `{ "facts": [ {measure, value, subject?, path?,
  line?, rule?, dims?} ] }` — one atomic fact per subject (never a baked total),
  each on a measure in the gauge's `emits`, calling the `files(glob)` /
  `ast_query(text, language, sexpr)` host builtins (see [collection.md](./collection.md)).
  A gauge may also return `"samples"`/`"findings"` (`GaugeFinding`, tsk311) —
  the legacy baked channel, kept for the built-in gauges until the read-flip; a
  facts-only gauge (the clean model) writes no baked run.
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
  - **user-global** — `global_config_dir()/{metrics,gauges,measures,dimensions}/*.yaml`,
    shared across projects, hot-reloaded by the config watcher. Global gauges +
    measures are active everywhere automatically; a global *metric* is enabled
    per-project with a `use:`.
  - **project** — `.oxplow/project.yaml` + gauge scripts under `oxplow/gauges/`.

  `use:` references a catalog metric key and layers threshold overrides; `key:`
  defines a new spec. `oxplow.*` is reserved for built-ins (a project may `use:`
  one but not `key:`-define under it). Gauges are definition-only (declared, never
  `use:`d).
- Validation mirrors the plugin rules: namespaced keys, project-relative
  `entryFile` (no `..`), known runtime/aggregation/displayKind/trigger/direction;
  a `key:` metric must set exactly one of `sourceMeasure`/`formula`; a `use:` with
  an unknown key resolves to a warning (skipped), not an error.

The in-oxplow agent authors these on request via the **`oxplow-metrics`** skill
+ the **`/oxplow:new-metric`** command (assets in `crates/oxplow-plugin/`,
materialized for Claude/Codex/opencode) — "make a metric that counts TODOs" →
the measure+gauge+metric trio + script + verification, no oxplow-team involvement.
`MetricsService::scaffold_metric` writes that trio (measure `<key>.count`, gauge
`<key>`, metric `<key>`) + a starter fact-emitting gauge script.

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
