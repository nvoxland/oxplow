---
description: Author an oxplow metric — a durable, chartable number tracked over time (count of X, complexity, bundle size, …). Scaffolds the .oxplow/project.yaml measure+gauge+metric trio + script and verifies it.
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

## 3. Otherwise define the trio (measure + gauge + metric)

Fastest path — the **`scaffold_metric` MCP tool**: it writes the measure +
gauge + metric trio into `.oxplow/project.yaml` (or the global config dir with
`scope: "global"`) plus a starter gauge script, and returns the script path.

```
scaffold_metric { key: "repo.todo_count", title: "TODO comments", language: "rust" }
→ { "script_path": "oxplow/gauges/repo_todo_count.star" }
```

The starter just counts TODO/FIXME per file — **open the returned script and
edit it** to compute what the user actually asked for (it can call
`files(glob)` / `ast_query(text, language, sexpr)` / `code_metrics(text,
language)`). Then jump to **Verify**.

By hand instead, add the trio (namespaced — `oxplow.*` is reserved) + a gauge
script under `oxplow/gauges/`:

```yaml
measures:
  - key: repo.todo_count           # the fact TYPE
    subjectKind: file
    unit: count
gauges:
  - key: repo.todo                 # the PRODUCER
    trigger: on-snapshot
    emits: [repo.todo_count]
    compute: { runtime: starlark, entryFile: oxplow/gauges/todo.star }
metrics:
  - key: repo.todo_count           # the SPEC
    title: "TODO comments"
    sourceMeasure: repo.todo_count
    aggregation: sum
    direction: lower-better
    unit: count
```

```python
# oxplow/gauges/todo.star — one per-file FACT (not a baked total)
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        c = 0
        for cm in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            c += len(regex_find(r"(?i)\b(TODO|FIXME)\b", cm["text"]))
        if c > 0:
            facts.append({"measure": "repo.todo_count", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"]})
    return {"facts": facts}
```

The gauge returns `{ "facts": [ { "measure", "value", "subject"?, "path"?, "dims"? } ] }`
— one atomic fact per subject; the `metrics:` spec (`aggregation: sum`) re-adds
them. Read the `oxplow-metrics` skill for the four-block model, the full builtin
surface (`files`/`ast_query`/`code_metrics`/`regex_find`/…) and the report-derived
+ exec patterns. The bundled scripts in
`crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/` are copy-paste
templates.

## 4. Verify

1. `run_metric { key: "repo.todo" }` (MCP) — runs the gauge now, returns the fact
   count.
2. `get_metric_summary { metric_key: "repo.todo_count" }` — confirm the value.
3. It now appears on the Metrics page automatically.

Leave the `.oxplow/project.yaml` + script diffs for the user to review (committed files).
