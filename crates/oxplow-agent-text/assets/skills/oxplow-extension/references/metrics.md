# Contributing metrics


An extension can add to the metric catalog with `measures:`, `metrics:`
and `dimensions:` in `extension.yaml`, plus **fact collectors** (a
`collectors:` entry with `facts:` instead of `entities:`) that record the
facts — in the same schema the project's own extension uses (the `oxplow-metrics`
skill and oxplow's `new-metric` slash command cover it).

```yaml
measures:
  - { key: acme.todo, title: TODO comments }
metrics:
  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum, direction: lower-better }
collectors:
  - id: acme.todo_scan
    runtime: starlark
    entry: collectors/todo.star
    trigger: { on: [snapshot.taken] }
    facts: [acme.todo]
```

- `entry` is inside the extension folder.
- An extension's fact collector runs `starlark` or `jaq` only. `exec` is
  refused (only an own extension's fact collectors — no `source.yaml`, e.g. `oxplow/extensions/project/` — may be `exec`); it gets
  no `env`, `credentials` or `network`. Its script reads the snapshot
  with `files()` / `ast_query()` and returns `{"facts": [...]}`. It may
  also read the semantic layer or ask a model when it declares them
  (`needs: [sql.read]`, `[ai.compute]`; `references/collectors.md` "Host services").
- A collector has `entities:` or `facts:`, never both.
- A `gauges:` block is an unknown key; write `collectors:`.
- `.oxplow/project.yaml` takes no definitions: the project's own measures,
  collectors and metrics go in `oxplow/extensions/project/extension.yaml`.
- Metrics are `key:` definitions and are on while the extension is
  enabled. A project can still turn one off or change its target with a
  `use:` entry in `.oxplow/project.yaml`. `use:` isn't allowed in an
  extension.
- Disabling the extension drops its metrics from the catalog; its
  measures and their history stay.
- `dimensions:` work the same, including entity dimensions
  (`{ key: acme.team, entity: v_acme_issue, expr: e.team }`), which
  slice entity metrics over the same view. `promote:` isn't allowed in an
  extension.
