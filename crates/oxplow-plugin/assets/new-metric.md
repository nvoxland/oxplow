---
description: Author an oxplow metric — a durable, chartable number tracked over time (count of X, complexity, bundle size, …). Scaffolds the .oxplow/project.yaml `metrics:` entry + script and verifies it.
---

Author a metric so oxplow tracks a number over time and charts it on the
**Metrics** page. The standing rules + full host-builtin reference live in the
`oxplow-metrics` skill — follow it. File a task first (you'll edit project
files), then:

## 1. Clarify the ask

Turn the user's request into one metric: *what number*, *over what* (the whole
tree? changed lines? a build artifact?), and *which direction is good*
(`lower-better` for "fewer TODOs", `higher-better` for coverage, `neutral` for
LOC). Ask only if genuinely ambiguous.

## 2. Prefer a built-in

Call `list_metric_definitions { scope: "built-in" }` (MCP). If a bundled metric
already measures it (unsafe blocks, unwrap/expect, TODO markers, function count,
high-complexity functions, `any` usage, …), just enable it — one line:

```yaml
metrics:
  - use: oxplow.rust.unsafe_blocks
    target: 0
```

## 3. Otherwise define a new one

Add a `key:` entry (namespaced — `oxplow.*` is reserved) + a script under
`oxplow/metrics/`:

```yaml
metrics:
  - key: repo.todo_count
    kind: gauge
    title: "TODO comments"
    direction: lower-better
    unit: count
    trigger: on-snapshot
    dimensions: [language]
    compute: { runtime: starlark, entryFile: oxplow/metrics/todo_count.star }
```

```python
# oxplow/metrics/todo_count.star
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        for c in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            n += len(regex_find(r"(?i)\b(TODO|FIXME)\b", c["text"]))
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
```

The gauge script returns `{ "samples": [ { "value", "subject"?, "dims"? } ] }`.
Read the `oxplow-metrics` skill for the full builtin surface
(`files`/`ast_query`/`code_metrics`/`regex_find`/…) and the report-derived + exec
patterns. The bundled scripts in
`crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/` are copy-paste
templates.

## 4. Verify

1. `run_metric { key: "repo.todo_count" }` (MCP) — runs it now, returns the
   sample count.
2. `get_metric_summary { metric_key: "repo.todo_count" }` — confirm the value.
3. It now appears on the Metrics page automatically.

Leave the `.oxplow/project.yaml` + script diffs for the user to review (committed files).
