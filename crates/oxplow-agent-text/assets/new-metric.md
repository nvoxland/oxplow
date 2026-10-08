---
description: Author an oxplow metric — a durable, chartable number tracked over time (count of X, complexity, bundle size, …). Scaffolds the .oxplow/project.yaml measure+collector+metric trio + script and verifies it.
---

Author a metric so oxplow tracks a number over time and charts it on the
**Metrics** page. The standing rules + full host-builtin reference live in the
`oxplow-metrics` skill — follow it. Then:

## 1. Clarify the ask

Turn the user's request into one metric: *what number*, *over what* (the whole
tree? changed lines? a build artifact?), and *which direction is good*
(`lower-better` for "fewer TODOs", `higher-better` for coverage, `neutral` for
LOC). Ask only if genuinely ambiguous.

## 2. Prefer a built-in

List the bundled metrics with `query_sql`:
`SELECT key, title, language, enabled FROM v_metric_catalog WHERE scope = 'built-in'`.
If one already measures it (unsafe blocks, unwrap/expect, TODO markers, function
count, high-complexity functions, `any` usage, …), just turn it on with the
`oxplow.metric.enable` command (`run_command`):
`{ "keys": ["oxplow.rust.unsafe_blocks"], "enabled": true }`.

## 3. Otherwise define the trio (measure + collector + metric)

Fastest path — the **`oxplow.metric.scaffold` command** (`run_command`). It writes
nothing: it returns a starter collector script and the measure + collector + metric
trio as a `.oxplow/project.yaml` snippet.

```
oxplow.metric.scaffold { key: "repo.todo_count", title: "TODO comments", language: "rust" }
→ { "scriptPath": "oxplow/collectors/repo_todo_count.star", "script": "…", "projectYaml": "measures: …" }
```

1. Write `script` at `scriptPath`, changed to compute what the user
   actually asked for (the starter just counts TODO/FIXME per file; it can
   call `files(glob)` / `ast_query(text, language, sexpr)` /
   `code_metrics(text, language)`).
2. Add each entry with `oxplow.config.set` (`run_command`): `oxplow.config.get` the
   `measures` / `collectors` / `metrics` list, append the new entry, and set the
   list back. `collectors` is person-only (a collector runs a program), so that
   `oxplow.config.set` asks the person to confirm. The catalog reseeds on the change.

Then jump to **Verify**.

The trio (namespaced — `oxplow.*` is reserved) and its collector script under
`oxplow/collectors/` look like this:

```yaml
measures:
  - key: repo.todo_count           # the fact TYPE
    subjectKind: file
    unit: count
collectors:
  - id: repo.todo                  # the PRODUCER
    doc: TODO comment scan
    runtime: starlark
    entry: oxplow/collectors/todo.star
    trigger: { on: [snapshot.taken] }
    facts: [repo.todo_count]       # the measures it may record
metrics:
  - key: repo.todo_count           # the SPEC
    title: "TODO comments"
    sourceMeasure: repo.todo_count
    aggregation: sum
    direction: lower-better
    unit: count
```

```python
# oxplow/collectors/todo.star — one per-file FACT (not a baked total)
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

The collector returns `{ "facts": [ { "measure", "value", "subject"?, "path"?, "dims"? } ] }`
— one atomic fact per subject; the `metrics:` spec (`aggregation: sum`) re-adds
them. Read the `oxplow-metrics` skill for the four-block model, the full builtin
surface (`files`/`ast_query`/`code_metrics`/`regex_find`/…) and the report-derived
+ exec patterns. The bundled scripts in
`crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/` are copy-paste
templates.

## 4. Verify

1. `oxplow.collector.sync { owner: "project", id: "repo.todo" }` (`run_command`) —
   runs the collector now, returns the `facts` count.
2. `query_sql`: `SELECT bucket, MEASURE('repo.todo_count') FROM metric_grid('day')`
   — confirm the value.
3. It now appears on the Metrics page automatically.

Leave the `.oxplow/project.yaml` + script diffs for the user to review (committed files).
