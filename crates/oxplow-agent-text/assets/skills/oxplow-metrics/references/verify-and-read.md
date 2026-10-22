# Verifying, reading metrics in SQL, and troubleshooting

## Step 5 — verify

1. **Check and test the extension:** `oxplow extension check <name>` (no
   errors, no warnings; it dry-runs the lens) and `oxplow extension test
   <name>` (the collector over the fixture returns the expected facts).
2. **Run the collector now:** the `oxplow.collector.sync` command,
   `{ "collector": "<extension name>/<ns>.scan" }` (`project/<id>`
   for a project collector) — runs it against the latest snapshot and records its
   facts; returns the `facts` count. `oxplow.metric.rebuild`
   (`{ "force": true }` to redo everything) runs every snapshot collector's
   whole-tree baseline.
3. **Read the metric back** with `oxplow_query_sql` and the metric function; slice it
   by your dimension:

   ```sql
   SELECT bucket, MEASURE('acme.todo_count') FROM metric_grid('day')
   SELECT bucket, "acme.area", MEASURE('acme.todo_count') FROM metric_grid('day', 'acme.area')
   ```

4. The metric appears on the **Metrics** page automatically — no UI code — and
   its lens in the launcher.

If the run fails the error says why: a fact on a measure the collector doesn't
list in `facts:` or no measure defines ("declare it in `measures:`"), or a
`dims` key that isn't namespaced ("isn't namespaced: … `<namespace>.<key>`").
Files over `snapshot_max_file_bytes` and binary files are not given to
collectors; the sweep reports them as excluded files.

**Reading metrics.** Metrics are read in SQL. `metric_grid(bucket[, dim])`
is a grid of time buckets (`day`, `week`, `month`, or `capture` for one row
per recording, with a `capture_id` that joins `v_capture`); each
`MEASURE('<key>')` is a metric's series on it, computed with the metric's
own aggregation and time semantics. The optional `dim` slices it: any
dimension the facts carry, or `oxplow.stream` / `oxplow.thread` /
`oxplow.effort` / `oxplow.work_item` (the effort's work-item ref) /
`oxplow.vcs_rev`. "Tokens per work item per week" is:

```sql
SELECT bucket, "oxplow.work_item", MEASURE('oxplow.tokens.total')
FROM metric_grid('week', 'oxplow.work_item')
```

**What's behind a metric** — the located items, as they stand now — is
`metric_findings('<key>')`; add a capture id for one recording's:

```sql
SELECT path, line, severity, rule, message, value
FROM metric_findings('oxplow.rust.unsafe_blocks') ORDER BY path, line
```

`severity` is the item's own (a lint's), else its value against the
metric's thresholds. The catalog and raw data are views: `v_metric_spec`
(definitions and thresholds), `v_metric_catalog` (what's available and
on), `v_measure`, `v_dimension`, `v_capture` and `v_fact` (every fact ever
recorded — history, including items since fixed).

For a CI-imported or agent-asserted number oxplow can't compute itself, use
the `oxplow.metric.record` command, `{ key, value, subject?, dims? }` (stored
`asserted`, lower-trust).

## Gotchas

- **Collectors record facts, metrics aggregate them.** The trio splits producer
  (`collectors:`), fact type (`measures:`) and read spec (`metrics:`). A collector script
  returns `{ "facts": [...] }` (one fact per subject), never a baked total.
- **Declare-to-collect.** A fact is dropped unless its measure is (a) declared in
  `measures:` (or a built-in) AND (b) in the collector's own `facts` list.
- **`oxplow.*` is oxplow's own** (its foundation extension's) — define new
  measures/collectors/metrics under a project/vendor namespace, in an extension.
- **`entry` is relative to the extension folder**, no leading `/`, no `..`.
- **No `gauges:` block.** It's an unknown key; write `collectors:`.
- **Scripts do no I/O** (jaq/starlark) — that's what keeps facts `observed`. Reach
  for `exec` only when you truly must shell out.
- A `use:` entry may only re-target thresholds; the measure/aggregation/filter are
  inherent to the definition.
- If a fact doesn't appear, check the run: every run logs a `collector.synced`
  event and a `collector_run` row (a failed one with its error) — or re-run
  `oxplow.collector.sync` and read the return.
