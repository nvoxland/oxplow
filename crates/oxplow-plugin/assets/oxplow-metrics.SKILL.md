---
name: oxplow-metrics
description: Author oxplow metrics on request — durable, BI-reportable numbers tracked over time (LOC, unsafe blocks, bundle size, TODO count, complexity, …). Loads when the user asks to "make/add/track a metric", "count X over time", "chart X", "set a target on X", or "measure X in the codebase". Teaches the four config blocks in .oxplow/project.yaml (measures / gauges / metrics / dimensions), the gauge script surface (files/ast_query/code_metrics), and how to verify.
---

# Authoring oxplow metrics

oxplow's metric substrate is **dimensional / BI-shaped**: gauges emit durable,
atomic **facts** on declared **measures**; **metrics** are read-time *specs* that
aggregate those facts. Facts outlive the effort, are branch/git-version stamped,
and a metric charts on the **Metrics** page with **no UI work**. When the user
asks to track/measure/chart something, author a metric. You don't need the oxplow
team — the authoring surface is public config + a small script.

## The four blocks

`.oxplow/project.yaml` has four orthogonal metric blocks, mirroring the substrate:

| block | what it declares | cardinality |
|---|---|---|
| `measures:` | a **fact TYPE** a gauge may emit (`acme.complexity`) | one measure ← many gauges |
| `gauges:` | a **fact PRODUCER** — runs a script, emits facts on measures it `emits` | one gauge → many measures |
| `metrics:` | a **read-time SPEC** — aggregates a measure (or a formula over metrics) | one measure → many metrics |
| `dimensions:` | a **conformed slice axis** for drill-across (`acme.rule`) | shared by many facts |

A metric no longer *computes* anything — it names a `sourceMeasure` + an
`aggregation`. The number comes from re-aggregating the facts a gauge emitted.

## Step 1 — is there already a built-in? (`use:`)

**Always prefer `use:`** when a bundled metric covers the ask — list them with the
`list_metric_definitions` MCP tool (scope `built-in`). Bundled Rust/TS/Clojure/C#
metrics exist for unsafe blocks, unwrap/expect, panic macros, TODO markers,
function count, high-complexity / long functions, `any` usage, non-null
assertions, console calls, ts-ignore, defn count, empty catch, blocking async, …
If one fits, a one-line `use:` is the whole job:

```yaml
metrics:
  - use: oxplow.rust.unsafe_blocks
    target: 0            # a use: may only re-target thresholds (target/warnAt/failAt)
```

`oxplow.*` is reserved for built-ins — a project may `use:` one but not `key:`-define under it.

## Step 2 — a new metric: declare the trio (measure + gauge + metric)

The fastest path is the `scaffold_metric` MCP tool — it writes all three entries
plus a starter gauge script, then reseeds. To do it by hand, add the trio:

```yaml
measures:
  - key: repo.todo_count            # the fact TYPE (per-file counts)
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive  # additivity OVER TIME (see below)

gauges:
  - key: repo.todo                  # the PRODUCER
    trigger: on-snapshot            # runs after each snapshot
    emits: [repo.todo_count]        # declare-to-collect: it may only emit these
    compute: { runtime: starlark, entryFile: oxplow/gauges/todo.star }

metrics:
  - key: repo.todo_count            # the SPEC (the chartable metric)
    title: "TODO comments"
    sourceMeasure: repo.todo_count
    aggregation: sum                # combine the per-file facts WITHIN a capture
    direction: lower-better
    unit: count
    displayKind: gauge
    sliceableDims: [language]
```

Namespace every key `<vendor>.<id>` (e.g. `repo.todo_count`, `acme.bundle_size`).

### `measures:` fields

| field | meaning |
|---|---|
| `key` | namespaced fact-type id (required) |
| `subjectKind` | what each fact is about: `file` \| `symbol` \| `test` \| `model` … |
| `unit` | `count` \| `%` \| `ms` \| `kb` \| `usd` … |
| `temporalSemantics` | additivity **over time**: `additive` (SUM across captures — events/tokens), `semi-additive` (take the LAST capture — stocks/counts + level ratios like coverage, default), `non-additive` (re-derive Σnum/Σden across all captures — accumulating ratios) |

### `metrics:` (spec) fields

| field | meaning |
|---|---|
| `key` / `use` | define a new spec / enable a catalog one (exactly one) |
| `sourceMeasure` | the measure whose facts this aggregates (required for a `key:` metric, unless `formula`) |
| `aggregation` | combine facts **within a capture**: `count` \| `sum` \| `avg` \| `min` \| `max` \| `last` \| `ratio` |
| `filter` | keep only matching facts before aggregating: `{ minValue?, severity?, dimEq?: [key, value] }` |
| `formula` | a derived metric over OTHER metrics: `{ op: add\|sub\|mul\|div, left: <metricKey>, right: <metricKey> }` (no `sourceMeasure`) |
| `direction` | `higher-better` \| `lower-better` \| `neutral` — red/green sense |
| `target` / `warnAt` / `failAt` | thresholds (via `direction`) — coloring + the Stop-hook nudge |
| `displayKind` | read presentation: `gauge` \| `findings` \| `test` \| `coverage` \| `event` |
| `category` | catalog grouping: `operational` \| `testing` \| `coverage` \| `static-quality` \| `custom` |
| `sliceableDims` | conformed-dimension keys for group-by / drill-across |

**Two-axis aggregation.** `aggregation` combines the facts inside one capture; the
source measure's `temporalSemantics` governs how the series collapses across
time. A count-over-threshold is `aggregation: count` + a `filter: { minValue: N }`.

### `gauges:` fields

| field | meaning |
|---|---|
| `key` | namespaced producer id (required) |
| `trigger` | when it runs: `on-snapshot` (after a snapshot — the default for tree scans), `on-effort-complete`, `manual` (only via `run_metric`). (`on-report` / `continuous` reserved.) |
| `emits` | the measure keys it may emit facts on (declare-to-collect — a fact outside this list is dropped) |
| `compute` | `{ runtime, input?, entryFile, args?, report? }` — how it produces facts |

## Step 3 — write the gauge script (emits FACTS)

A gauge script is Starlark (or jaq/exec) that returns the **fact shape** — one
atomic fact per subject, NOT a pre-aggregated total:

```json
{ "facts": [ { "measure": "repo.todo_count", "value": <n>, "subject"?: "file:src/a.rs",
               "path"?: "src/a.rs", "line"?: 12, "rule"?: "todo", "dims"?: { ... } } ] }
```

- `measure` (required) is the measure key — MUST be in the gauge's `emits`.
- `value` is the atomic number for this subject.
- `subject` is a `"kind:ref"` string (`file:src/a.rs`, `symbol:src/a.rs::foo`).
- `rule` is a conformed slice value read as the `oxplow.rule` dimension (so a spec
  can `filter: { dimEq: [oxplow.rule, …] }` or slice by it).
- `num`/`den` (optional) are ratio components — supply them when the measure is a
  ratio base so a `aggregation: ratio` metric re-derives Σnum/Σden (coverage %,
  pass rate) instead of averaging pre-divided values.

The Starlark entry is `def transform(input): … return { "facts": [...] }`.

### Three authoring patterns

**A) Tree-derived (the common case)** — scan the snapshot. Host builtins:

- `files(glob)` → `[{path, text}]` of snapshot files (deterministic → `observed`).
- `ast_query(text, language, sexpr)` → flat tree-sitter matches. Languages:
  `rust`/`typescript`/`tsx`/`javascript`/`python`/`go`/`java`/`c`/`cpp`/`clojure`.
- `code_metrics(text, language)` → per-function `[{name, complexity, length,
  parameter_count, start_line, end_line, visibility}]`.
- plus `regex_find`, `parse_json`, `parse_xml`, `lines`, `lcov_records`, `xpath`.

```python
# oxplow/gauges/todo.star — one per-file fact on repo.todo_count
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        c = 0
        for cm in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            c += len(regex_find(r"(?i)\b(TODO|FIXME)\b", cm["text"]))
        if c > 0:
            facts.append({"measure": "repo.todo_count", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"language": "rust"}})
    return {"facts": facts}
```

The `metrics:` spec `aggregation: sum` re-adds the per-file facts into the headline.

**B) Report-derived** — reshape a build/tool report with jaq or starlark:
`compute: { runtime: jaq, input: json, entryFile: …jq, report: path/to/report.json }`.

**C) exec (escape hatch)** — a program that prints the fact JSON to stdout:
`compute: { runtime: exec, entryFile: oxplow/gauges/bundle-size.sh }`. Lower trust
(it does I/O) — tagged `plugin-exec:<key>`. Use only when no in-process tier can
compute it.

The bundled gauge scripts in
`crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/*.star` are the canonical
copy-paste templates — each emits per-item facts.

## Step 4 — scope

- **Project** (default): `.oxplow/project.yaml` + scripts under `oxplow/gauges/`
  (checked into the repo, shared with the team).
- **User-global** (cross-project): `*.yaml` files under the user's global config
  dir — a `measures/`, `gauges/`, and `metrics/` folder — hot-reloaded. Global
  measures + gauges are active in every project automatically; a global *metric*
  is enabled per-project with a `use:`.

Precedence is **project > global > built-in** by key.

## Step 5 — verify

1. **Run the gauge now:** `run_metric { key: "repo.todo" }` (MCP) — runs the gauge
   against the latest snapshot and records its facts; returns the count.
2. **Read the metric back:** `list_metric_samples { metric_key: "repo.todo_count" }`
   or `get_metric_summary { metric_key: "repo.todo_count" }`.
3. The metric appears on the **Metrics** page automatically — no UI code.

For a CI-imported or agent-asserted number oxplow can't compute itself, use
`record_metric { key, value, subject?, dims? }` (stored `asserted`, lower-trust).

## Gotchas

- **Gauges emit facts, metrics aggregate them.** The trio splits producer
  (`gauges:`), fact type (`measures:`) and read spec (`metrics:`). A gauge script
  returns `{ "facts": [...] }` (one fact per subject), never a baked total.
- **Declare-to-collect.** A fact is dropped unless its measure is (a) declared in
  `measures:` (or a built-in) AND (b) in the gauge's own `emits` list.
- **`oxplow.*` is reserved** — define new measures/gauges/metrics under a
  project/vendor namespace.
- **entryFile is project-relative**, no leading `/`, no `..`.
- **Scripts do no I/O** (jaq/starlark) — that's what keeps facts `observed`. Reach
  for `exec` only when you truly must shell out.
- A `use:` entry may only re-target thresholds; the measure/aggregation/filter are
  inherent to the definition.
- If a fact doesn't appear, the run is best-effort (errors are logged, not
  surfaced) — check the daemon log, or re-run `run_metric` and read the return.
