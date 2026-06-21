# Metrics — the unified metric substrate

What this doc covers: oxplow's **metric substrate** (epic tsk213) — one durable,
typed model for any deterministically-computable number tracked over time, the
successor to `effort_observation` and (eventually) `code_quality_*`. Coverage,
tests, clippy findings, tokens, cost — all become *metric definitions* over the
same tables, queryable by the agent (MCP) and the renderer (IPC), and surfaced
on the Metrics page.

Status: substrate + read surface are **live**; the legacy `effort_observation`
path still runs **alongside** (dual-write) until the UI fully moves over and it's
dropped. See `tsk215`/`tsk219` for the remaining work.

## Why it exists

`effort_observation` (see [collection.md](./collection.md)) was the first cut,
but it's **effort-scoped, CASCADE-deleted with its effort, and pruned to the
last 10** — so it can't answer "how did coverage move over the last month" or
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

Producers are the only thing that writes samples. Today they're **dual-write
side-bands** that also feed the legacy tables (best-effort: a metric write error
is logged via `tracing::warn!`, never fails the host path):

| producer | where | emits |
|---|---|---|
| coverage / tests / analysis | `crates/oxplow-app/src/collection.rs` (`mirror_coverage_metric` / `mirror_test_metrics` / `mirror_analysis_metrics`, called from the existing `store_diff_coverage`/`record_test_run`/`record_static_analysis`) | `oxplow.coverage.diff_pct`; `oxplow.tests.{passed,failed,total}`; `oxplow.analysis.{errors,warnings}` + a finding per lint hit |
| token-parse | `crates/oxplow-app/src/token_usage.rs` (`project_token_metrics`, called from `on_stop`) | per-model `agent.tokens.{input,output,total}`, `agent.turns`, derived `agent.cost_usd` (per-model price table) |
| effort-lifecycle | `crates/oxplow-app/src/task_service.rs` (`project_effort_lifecycle_metrics`, called when `update()` closes an effort on an `in_progress` exit) | derived `effort.cycle_time_ms` (close − start, subject=effort) + `task.efforts` (efforts-so-far, the redo-rate signal) from `task_effort`; branch captured when the stream has a worktree |
| nudges | `crates/oxplow-app/src/collection.rs` (`project_nudge_metric`, called from `persist_nudge` after a fired nudge records) | `agent.nudges.fired` (event kind, run-less; value 1, subject=the nudge `kind`) — an agent-activity signal |

> Navigation / activity (`page_visit`, `usage_event`) are **deliberately not
> projected** into the substrate: they're oxplow-usage telemetry (UI metadata),
> not code or agent-activity metrics, so they stay in their own tables.

Each producer: `upsert_definition` (idempotent) → `record_run` → `record_sample`(s)
→ emit `OxplowEvent::MetricSamplesChanged { stream_id }`.

> **The plan is for these to become bundled plugins** (jaq/Starlark/exec,
> registered via `with_builtins()`) so producers are *content*, not hardcoded
> Rust — and for the legacy `effort_observation` path to be dropped (tsk215,
> tsk218). The hardcoded mirror helpers are the interim.

## Read surface

- **MCP** (`crates/oxplow-mcp/src/lib.rs`): `list_metric_definitions`
  (optional language/scope filter) + `list_metric_samples` (by key, newest-
  first). Lets the in-oxplow agent inspect metrics.
- **IPC** (`crates/oxplow-rpc/src/commands/metrics.rs` cores +
  `crates/oxplow-tauri-ipc/src/commands/metrics.rs` Tauri adapters, registered
  in `collect_commands!` + the remote `rpc_dispatch!`): same two reads, exposed
  to the renderer with generated TS bindings (`MetricDefinition`/`MetricSample`).
- **Event**: `OxplowEvent::MetricSamplesChanged { stream_id }` (coarse — the
  renderer refetches).

## UI

`apps/desktop/src/pages/MetricsPage.tsx` — a read-only **Metrics** page reachable
from the RailHud (Activity → Metrics; registered like the `usage` index page in
`tabState.PageKind`, `pageRefs.indexRef`, `RailHud/sections.ts`, `App.tsx`, and
`pageKinds.tsx`). Lists the catalog with each metric's latest value, a trend
sparkline, capture branch, and sample count; colored by target+direction;
live-refreshes on `metricSamplesChanged`. This is the **seed** of the full P4
Explorer/Catalog (multi-measure overlay, group-by dimensions, drill-across,
enable/scaffold).

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
  (see [collection.md](./collection.md)).
- **Three scopes**, precedence **project > global > built-in** by key: built-in
  (bundled — tsk218), user-global (`global_config_dir()/metrics/*.yaml`, shared
  across projects, hot-reloaded by the config watcher), and project
  (`oxplow.yaml`). `use:` references a catalog key and layers overrides; `key:`
  defines a new one. `oxplow.*` is reserved for built-ins.
- Validation mirrors the plugin rules: namespaced keys, project-relative
  `entryFile` (no `..`), known runtime/kind/trigger/direction; a `use:` with an
  unknown key resolves to a warning (skipped), not an error.

(The agent-facing skill that authors these on request is tsk221.)

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
