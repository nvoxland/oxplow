# Writing the collector script (records facts)

## Step 3 — write the collector script (records FACTS)

A fact collector's script is Starlark (or jaq/exec) that returns the **fact
shape** — one atomic fact per subject, NOT a pre-aggregated total:

```json
{ "facts": [ { "measure": "repo.todo_count", "value": <n>, "subject"?: "file:src/a.rs",
               "path"?: "src/a.rs", "line"?: 12, "rule"?: "todo", "dims"?: { ... } } ] }
```

- `measure` (required) is the measure key — MUST be in the collector's `facts`.
- `dims` keys are namespaced: a conformed dimension by its `oxplow.*` key
  (`oxplow.language`), one of your own `<namespace>.<name>`. A bare key
  (`language`) is refused, and the error names the conformed one.
- `value` is the atomic number for this subject.
- `subject` is one URI, built with `uri.build` (`oxplow:oxplow/file?path=src/a.rs`, `oxplow:oxplow/symbol?path=src/a.rs&symbol=foo`). A
  subject that is not a URI is refused (the run fails, naming the measure and the string).
- `rule` is a conformed slice value read as the `oxplow.rule` dimension (so a spec
  can `filter: { dimEq: [oxplow.rule, …] }` or slice by it).
- `num`/`den` (optional) are ratio components — supply them when the measure is a
  ratio base so a `aggregation: ratio` metric re-derives Σnum/Σden (coverage %,
  pass rate) instead of averaging pre-divided values.

The Starlark entry is `def transform(input): … return { "facts": [...] }`.
`input` is `{report?, rows?, event?}`: the parsed `report`, the `input:`
query's rows, and the event that triggered the run. Anything but `facts` in
the return (an old `samples` / `findings` shape) is refused.

### Three authoring patterns

**A) Tree-derived (the common case)** — scan the snapshot. Host builtins:

- `files(glob)` → `[{path, text}]` of snapshot files (deterministic → `observed`).
- `ast_query(text, language, sexpr)` → flat tree-sitter matches. Languages:
  `rust`/`typescript`/`tsx`/`javascript`/`python`/`go`/`java`/`c`/`cpp`/`clojure`.
- `code_metrics(text, language, path)` → per-function `[{name, id, uri,
  container, kind, complexity, length, parameter_count, start_line,
  end_line, visibility, has_doc}]`. `uri` is the function's
  `oxplow:oxplow/symbol?path=<path>&symbol=<id>` URI: use it as a per-function fact's `subject`.
- `duplicate_blocks(min_lines)` → every duplicated block pair among the
  snapshot files `[{a_path, a_start_line, a_end_line, b_path, b_start_line,
  b_end_line, line_count}]` (see oxplow-foundation's `oxplow.duplicate_lines`).
- plus `regex_find`, `parse_json`, `parse_xml`, `lines`, `lcov_records`, `xpath`.
- A fact collector can't call the `ai_*` builtins (those are for entity
  collectors, which in turn get no `files()`).

```python
# .oxplow/extensions/project/collectors/todo.star — one per-file fact on repo.todo_count
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        c = 0
        for cm in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            c += len(regex_find(r"(?i)\b(TODO|FIXME)\b", cm["text"]))
        if c > 0:
            facts.append({"measure": "repo.todo_count", "value": c,
                          "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
```

The `metrics:` spec `aggregation: sum` re-adds the per-file facts into the headline.

**B) Report-derived** — reshape a build/tool report with jaq or starlark:
`runtime: jaq`, `entry: …jq`, `report: { path: path/to/report.json, format: json }`;
the parsed report is `input.report`.

**C) exec (escape hatch)** — a program that prints the fact JSON to stdout:
`runtime: exec`, `entry: collectors/bundle-size.sh` (in `.oxplow/extensions/project/`). Lower trust (it does
I/O) — tagged `exec:<id>`. Only a non-bundled extension's collectors (e.g. the project's
`.oxplow/extensions/project/`) may be `exec` (a bundled extension's fact
collector is starlark or jaq), and it gets no
`env` / `credentials` / `network`. Use it only when no in-process tier can
compute it: the extension must declare `permissions: { process: true }`
(plus the `network` hosts and `secrets` it uses), and **the user approves that
when the extension is deployed** (you can't; your deploy that adds a permission
becomes a proposal). A change to the program is a redeploy.

The collector scripts that `$OXPLOW_BIN extension new metric --language <lang>`
scaffolds are the canonical copy-paste templates — each emits per-item
facts.
