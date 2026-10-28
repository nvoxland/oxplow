---
name: oxplow-metrics
description: Author and read oxplow metrics — durable, BI-reportable numbers tracked over time (LOC, unsafe blocks, bundle size, TODO count, complexity, …). Loads when the user asks to "make/add/track a metric", "count X over time", "chart X", "set a target on X", or "measure X in the codebase". Teaches the metric extension (oxplow.metric.scaffold), its four blocks (measures / collectors / metrics / dimensions), the fact-collector script surface, oxplow.collector.sync and metric_grid().
---

# Authoring oxplow metrics

oxplow's metric substrate is **dimensional / BI-shaped**: collectors record durable,
atomic **facts** on declared **measures**; **metrics** are read-time *specs* that
aggregate those facts. Facts outlive the effort, are branch/git-version stamped,
and a metric charts on the **Metrics** page with **no UI work**. When the user
asks to track/measure/chart something, author a metric.

## The four blocks

An extension's `extension.yaml` (a code metric you author, or the project's own
extension, `.oxplow/extensions/project/extension.yaml`) has four orthogonal metric blocks, mirroring the substrate:

| block | what it declares | cardinality |
|---|---|---|
| `measures:` | a **fact TYPE** a collector may record (`acme.complexity`) | one measure ← many collectors |
| `collectors:` | a **fact PRODUCER** — runs a script, records facts on the measures in its `facts:` | one collector → many measures |
| `metrics:` | a **read-time SPEC** — aggregates a measure (or a formula over metrics) | one measure → many metrics |
| `dimensions:` | a **conformed slice axis** for drill-across (`acme.rule`) | shared by many facts |

A metric no longer *computes* anything — it names a `sourceMeasure` + an
`aggregation`. The number comes from re-aggregating the facts a collector recorded.

## Step 1 — is there already a metric? (`use:`)

**Always prefer `use:`** when a bundled metric covers the ask — list them with
`oxplow_query_sql`:

```sql
SELECT key, title, language, enabled FROM v_metric_catalog WHERE scope = 'extension:oxplow-foundation'
```
 Bundled Rust/TS/Clojure/C#
metrics exist for unsafe blocks, unwrap/expect, panic macros, TODO markers,
function count, high-complexity / long functions, `any` usage, non-null
assertions, console calls, ts-ignore, defn count, empty catch, blocking async, …
If one fits, turn it on with the `oxplow.metric.enable` command (through `run_command`):
`{ "keys": ["oxplow.rust.unsafe_blocks"], "enabled": true }`. That writes the
`use:` line for you:

```yaml
metrics:
  - use: oxplow.rust.unsafe_blocks
    target: 0            # a use: may only re-target thresholds (target/warnAt/failAt)
```

`oxplow.*` is oxplow's own — a project may `use:` one but not `key:`-define under it.

## Where the detail is

Open a reference when you reach its step: `references/<name>.md` in a
harness with skill files, or `oxplow_get_skill { name: "oxplow-metrics", reference:
"<name>" }` without.

- `references/declaring.md` (`declaring`) — **writing the metric.**
  `oxplow.metric.scaffold` and the files it returns, the `measures:` /
  `metrics:` / `collectors:` / `dimensions:` YAML and every field, the
  project's own extension, and where a metric lives (extension, project,
  user-global; precedence).
- `references/collector-scripts.md` (`collector-scripts`) — **the script.**
  The fact shape a collector returns, `files` / `ast_query` /
  `code_metrics` and the other host builtins, and the tree-derived,
  report-derived and `exec` patterns.
- `references/verify-and-read.md` (`verify-and-read`) — **proving it and
  reading it.** `extension check` / `test`, `oxplow.collector.sync`,
  `metric_grid`, `metric_findings`, `v_metric_point`, `v_metric_*`,
  `oxplow.metric.record`, and the gotchas when a fact doesn't appear.
- `references/entity-metrics.md` (`entity-metrics`) — **a number about
  records oxplow already has** (work items, commits, test runs): no
  collector, just an aggregate over a `v_*` model. Which models exist is
  oxplow-extension's (`v_dataset`).
