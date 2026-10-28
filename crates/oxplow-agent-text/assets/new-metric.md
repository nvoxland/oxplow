---
description: Author an oxplow metric — a durable, chartable number tracked over time (count of X, complexity, bundle size, …). Scaffolds an extension (measure + collector + dimension + metric + page) and verifies it.
---

Author a metric so oxplow tracks a number over time and charts it on the
**Metrics** page. The standing rules + full host-builtin reference live in the
`oxplow-metrics` skill — follow it. Then:

## 1. Clarify the ask

Turn the user's request into one metric: *what number*, *over what* (the whole
tree? changed lines? a build artifact?), and *which direction is good*
(`lower-better` for "fewer TODOs", `higher-better` for coverage, `neutral` for
LOC). Ask only if genuinely ambiguous.

## 2. Prefer an existing metric

List the bundled metrics with `oxplow_query_sql`:
`SELECT key, title, language, enabled FROM v_metric_catalog WHERE scope = 'extension:oxplow-foundation'`.
If one already measures it (unsafe blocks, unwrap/expect, TODO markers, function
count, high-complexity functions, `any` usage, …), just turn it on with the
`oxplow.metric.enable` command (`run_command`):
`{ "keys": ["oxplow.rust.unsafe_blocks"], "enabled": true }`.

## 3. Otherwise scaffold a metric extension

A metric you author is an **extension** under
`.oxplow/extensions/<name>/`: it declares the measure, the collector, the
dimension its facts carry, the metric and a page that links to the files,
and it has a fixture that proves the collector. Scaffold it with the
**`oxplow.metric.scaffold` command** (`run_command`), which writes nothing and
returns the files:

```
oxplow.metric.scaffold { name: "complex-fns", language: "rust", glob: "src/**/*.rs", origin: "<your effort ref>" }
→ { "name": "complex-fns", "dir": ".oxplow/extensions/complex-fns",
    "files": [ { "path": ".oxplow/extensions/complex-fns/extension.yaml", "content": "…" }, … ] }
```

(From a terminal, `$OXPLOW_BIN extension new metric complex-fns --language rust
--glob 'src/**/*.rs' --origin <ref>` writes the same files.) Write each file
with your file tools, then change them into what the user asked for:

- `collectors/complex_functions.star` — the starter counts each file's
  functions over a complexity `LIMIT` with `source_files()` and
  `code_metrics()`, and records **one fact per file** (`value`, `path`, the
  worst function's `line`, and `dims: { <ns>.area: <top folder> }`). Change
  what it counts; keep one atomic fact per subject, never a total.
- `extension.yaml` — the `measures:`, `dimensions:`, `collectors:` and
  `metrics:` blocks (namespaced `<ns>.*`, the name with `_` for `-`); set
  `direction`, `target`/`warnAt`/`failAt`, and write the `description` and
  `intent.purpose` TODOs.
- `fixtures/basic.yaml` and the intent example — a small corpus
  (`files: { path: text }`) and the facts it must produce. Change them with
  the script; they are the proof the collector is right.
- `pages/<name>.yaml` — a launcher page over `metric_findings('<metric>')`
  whose `path` column links to the file at the `line` column.

The trio in `extension.yaml` looks like this (all of it is generated):

```yaml
measures:
  - key: complex_fns.complex_functions   # the fact TYPE
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive     # the latest count is the answer
    fold: last                           # a metric over it reads the latest capture
    captureScope: per-path               # a snapshot rescans only the files it changed
dimensions:
  - key: complex_fns.area                # the slice axis the facts' dims use
collectors:
  - id: complex_fns.scan                 # the PRODUCER
    runtime: starlark
    entry: collectors/complex_functions.star
    trigger: { on: [snapshot.taken] }
    facts: [complex_fns.complex_functions]
metrics:
  - key: complex_fns.complex_functions   # the SPEC
    sourceMeasure: complex_fns.complex_functions
    aggregation: sum
    direction: lower-better
    displayKind: findings
    sliceableDims: [complex_fns.area]
```

Read the `oxplow-metrics` skill for the four-block model, the full builtin
surface (`files`/`ast_query`/`code_metrics`/`regex_find`/…) and when a
project-level `exec` or report-derived collector is the right home instead.
The scripts that `$OXPLOW_BIN extension new metric --language <lang>`
scaffolds are copy-paste templates.

## 4. Verify

1. `$OXPLOW_BIN extension check <name>` — no errors and no warnings (or
   `oxplow_validate_extension`). It dry-runs the page, so a bad query shows here.
2. `$OXPLOW_BIN extension test <name>` — the collector runs over the fixture's
   files and must return the facts the example expects.
3. Deploy it: `oxplow.extension.deploy { name: "<name>" }` (`run_command`). It returns once the extension is live. Then `oxplow.collector.sync { collector:
   "<name>/<ns>.scan" }` (`run_command`) runs the collector now and returns the
   `facts` count.
4. `oxplow_query_sql`: `SELECT bucket, MEASURE('<ns>.complex_functions') FROM
   metric_grid('day')` — confirm the value; slice it with `metric_grid('day',
   '<ns>.area')`.
5. It appears on the Metrics page automatically, and the page in the launcher
   (Code).

Leave the new extension folder for the user to review. It stays personal until they share it (`oxplow.extension.share { name }`).
