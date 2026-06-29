---
name: oxplow-metrics
description: Author oxplow metrics on request — durable, BI-reportable numbers tracked over time (LOC, unsafe blocks, bundle size, TODO count, complexity, …). Loads when the user asks to "make/add/track a metric", "count X over time", "chart X", "set a target on X", or "measure X in the codebase". Teaches the `metrics:` block in .oxplow/project.yaml, the gauge script surface (files/ast_query/code_metrics), and how to verify.
---

# Authoring oxplow metrics

A **metric** is a deterministically-computable number oxplow records over time
into the metric substrate — durable (it outlives the effort), branch/git-version
stamped, and chartable on the **Metrics** page with **no UI work**. When the user
asks to track/measure/chart something about the codebase or process, author a
metric. You don't need the oxplow team — the authoring surface is public config +
a small script.

## The shape of the work

Two artifacts:

1. A **`metrics:` entry** in the project's `.oxplow/project.yaml`.
2. (For a *new* metric) a small **script** under `oxplow/metrics/<name>.star`.

Then verify with the `run_metric` MCP tool and read it back.

## Step 1 — pick the form: `use:` vs `key:`

The top-level `metrics:` block holds entries in one of two forms:

```yaml
metrics:
  # use: enable an existing BUILT-IN metric (optionally override its target/trigger)
  - use: oxplow.rust.unsafe_blocks
    target: 0
  # key: DEFINE a new project metric (full definition + a compute script)
  - key: repo.todo_count
    kind: gauge
    title: "TODO comments"
    direction: lower-better
    unit: count
    trigger: on-snapshot
    dimensions: [language]
    compute: { runtime: starlark, entryFile: oxplow/metrics/todo_count.star }
```

- **Always prefer `use:`** when a bundled metric already covers the ask — list
  them with the `list_metric_definitions` MCP tool (scope `built-in`). Bundled
  Rust/TS/Clojure metrics exist for unsafe blocks, unwrap/expect calls, panic
  macros, TODO markers, function count, high-complexity / long functions, `any`
  usage, non-null assertions, console calls, ts-ignore, defn count, … If one
  fits, a one-line `use:` is the whole job.
- **`key:` defines a new metric.** Namespace it `<vendor>.<id>` (e.g.
  `repo.todo_count`, `acme.bundle_size`). **`oxplow.*` is reserved for built-ins**
  — a project `key:` under `oxplow.` is rejected.

### Definition fields (`key:` form)

| field | meaning |
|---|---|
| `key` | namespaced id (required) |
| `kind` | **`gauge`** — the author-able kind. (findings/test/coverage/event are produced by built-in collectors, not `metrics:` entries.) |
| `title` | display name |
| `unit` | `count` \| `%` \| `ms` \| `kb` \| `usd` … |
| `direction` | `higher-better` \| `lower-better` \| `neutral` — sets the red/green sense |
| `target` / `warnAt` / `failAt` | thresholds (interpreted via `direction`) — drive coloring + the Stop-hook nudge |
| `dimensions` | conformed-dimension keys the samples carry (e.g. `[language, git_version]`) — power group-by / drill-across |
| `trigger` | when it runs: `on-snapshot` (after a snapshot — the default for tree scans), `on-effort-complete` (when an effort closes), `manual` (only via `run_metric`). (`on-report` / `continuous` are reserved.) |
| `compute` | `{ runtime, input?, entryFile, args?, report? }` — how it computes |

## Step 2 — write the compute script (gauge)

A gauge script is Starlark (or jaq/exec) that returns the **gauge shape**:

```json
{ "samples": [ { "value": <number>, "subject"?: "kind:ref", "dims"?: { ... } } ] }
```

- `value` is the number recorded.
- `subject` (optional) is a `"kind:ref"` string, e.g. `"file:src/a.rs"`,
  `"module:apps/desktop"`, `"tree:."`.
- `dims` (optional) are extra dimensions stored on the sample.

The Starlark entry is `def transform(input): … return { "samples": [...] }`. For a
tree scan, `input` is unused — the script reads the snapshot via host builtins.

### Three authoring patterns

**A) Tree-derived (the common case)** — scan the snapshot. Host builtins:

- `files(glob)` → `[{path, text}]` of snapshot files matching the glob
  (deterministic, content-addressed → trusted as `observed`).
- `ast_query(text, language, sexpr)` → flat tree-sitter matches
  `[{capture, text, start_row, start_col, end_row, end_col}]`. Languages:
  `rust`/`typescript`/`tsx`/`javascript`/`python`/`go`/`java`/`c`/`cpp`/`clojure`.
- `code_metrics(text, language)` → per-function `[{name, complexity, length,
  parameter_count, start_line, end_line, visibility}]` (cyclomatic complexity,
  length, params via tree-sitter).
- plus `regex_find(pattern, text)`, `parse_json`, `parse_xml`, `lines`,
  `lcov_records`, `xpath`.

```python
# oxplow/metrics/todo_count.star — count TODO/FIXME in comments only
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        for c in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            n += len(regex_find(r"(?i)\b(TODO|FIXME)\b", c["text"]))
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
```

`compute: { runtime: starlark, entryFile: oxplow/metrics/todo_count.star }`,
`trigger: on-snapshot`.

**B) Report-derived** — reshape a build/tool report file with jaq or starlark.
Set `compute: { runtime: jaq, input: json, entryFile: …jq, report: path/to/report.json }`;
the host reads `report`, pre-parses per `input` (`text|json|xml|lcov|lines`), and
your script maps it to the gauge shape.

**C) exec (escape hatch)** — a program that prints the gauge JSON to stdout:
`compute: { runtime: exec, entryFile: oxplow/metrics/bundle-size.sh }`. Lower
trust (it does I/O) — tagged `plugin-exec:<key>`. Use for things no in-process
tier can compute (e.g. `du -k dist/bundle.js`).

The bundled metric scripts in oxplow's own
`crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/*.star` are the canonical
copy-paste templates.

## Step 3 — scope

- **Project** (default): `.oxplow/project.yaml` + scripts under `oxplow/metrics/` (checked
  into the repo, shared with the team).
- **User-global** (cross-project): a `*.yaml` under the user's global config dir
  `metrics/` folder (`{ metrics: [ … ] }`), hot-reloaded. Use when the user wants
  the metric in *all* their projects.

Precedence is **project > global > built-in** by key.

## Step 4 — verify

1. **Run it now:** `run_metric { key: "repo.todo_count" }` (MCP) — computes against
   the stream's latest snapshot and records samples; returns the count recorded.
   (`on-snapshot` metrics also run automatically on the next snapshot; `run_metric`
   is the on-demand way to check immediately.)
2. **Read it back:** `list_metric_samples { metric_key: "repo.todo_count" }` or
   `get_metric_summary { metric_key: "repo.todo_count" }` (latest value + delta vs
   target).
3. The metric now appears on the **Metrics** page automatically — no UI code.

For a CI-imported or agent-asserted number oxplow can't compute itself, use
`record_metric { key, value, subject?, dims? }` (stored `asserted`, lower-trust).

## Gotchas

- **Gauge is the kind for authored metrics.** A `metrics:` entry runs as a gauge;
  the script must return `{ "samples": [...] }`.
- **`oxplow.*` is reserved** — define new metrics under a project/vendor namespace.
- **entryFile is project-relative**, no leading `/`, no `..`.
- **Scripts do no I/O** (jaq/starlark) — that's what keeps them deterministic and
  `observed`. Reach for `exec` only when you truly must shell out.
- If a sample doesn't appear, the run is best-effort (errors are logged, not
  surfaced) — check the daemon log, or re-run with `run_metric` and read the
  return value.
