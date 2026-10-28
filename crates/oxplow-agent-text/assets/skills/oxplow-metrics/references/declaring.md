# Declaring a metric: scaffold, the four blocks, fields, scope

## Step 2 — a new metric: scaffold a metric extension

A metric you author is an **extension** (a source under `.oxplow/extensions/<name>/`; it runs after `oxplow.extension.deploy { name }`). Get it from the
`oxplow.metric.scaffold` command (`run_command`), which writes nothing and
returns the files (`$OXPLOW_BIN extension new metric <name> [--language rust]
[--glob '**/*.rs'] [--origin <your effort ref>]` writes the same ones from a
terminal):

```
oxplow.metric.scaffold { "name": "complex-fns", "language": "rust", "glob": "src/**/*.rs", "origin": "<your effort ref>" }
→ { "name": "complex-fns", "dir": ".oxplow/extensions/complex-fns", "files": [ { "path", "content" }, … ] }
```

Write each file with your file tools (the write guard applies), then change
them into the metric the user asked for. The extension declares all four
blocks, namespaced `<ns>.*` (the name with `_` for `-`), a launcher page and
a fixture that proves the collector:

| file | what it is |
|---|---|
| `extension.yaml` | `measures:` (one per-path, semi-additive measure on files), `dimensions:` (`<ns>.area`), `collectors:` (a Starlark one on `snapshot.taken`), `metrics:` (the `sum`, `lower-better`, `displayKind: findings`), and the `intent` with its example |
| `collectors/complex_functions.star` | counts each file's functions over a complexity `LIMIT` (`source_files()` + `code_metrics()`) and records **one fact per file**: `value`, `path`, the worst function's `line`, `dims: { <ns>.area: <top folder> }` |
| `pages/<name>.yaml` | a Code launcher page over `metric_findings('<metric>')`; its `path` column links `file` at the `line` column |
| `fixtures/basic.yaml` | the example as a fixture: `when: { collector, files: { path: text } }`, `expect: { facts: [...] }` |

Change the collector to what the user asked for and **change the fixture with
it** (a small corpus and the exact facts it yields); `extension test` runs the
collector over the fixture and fails when the facts differ, so it is the proof.
`$OXPLOW_BIN extension check <name>` must report no errors and no warnings. The
blocks look like this:

```yaml
measures:
  - key: acme.todo_count            # the fact TYPE (per-file counts)
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive  # additivity OVER TIME (see below)
    fold: last                        # how the series becomes one number (required for a metric)
    captureScope: per-path            # a snapshot rescans only the files it changed

dimensions:
  - key: acme.area                  # a slice axis your facts' dims use
    label: Area

collectors:
  - id: acme.todo                   # the PRODUCER
    doc: TODO comment scan
    runtime: starlark
    entry: collectors/todo.star     # relative to the extension
    on: [snapshot.taken]   # runs after each snapshot that recorded files
    facts: [acme.todo_count]        # declare-to-collect: it may only record these

metrics:
  - key: acme.todo_count            # the SPEC (the chartable metric)
    title: "TODO comments"
    measure: acme.todo_count
    expression: "SUM(value)"        # combine the per-file facts WITHIN a capture
    direction: lower-better
    unit: count
    displayKind: findings           # located facts: the drill-in lists the files
    sliceableDims: [acme.area]
```

The project's own metrics (a report-derived or `exec` collector) go in the same
four blocks of its own extension, `.oxplow/extensions/project/extension.yaml` —
`.oxplow/project.yaml` takes no `collectors:`, `measures:`, `dimensions:` or
`metrics:` definitions, only `use:` entries and `enabled: false` markers. Create
the file with the `manifest: 2` / `name: project` / `intent:` header when it is
missing; the collector `entry` is relative to the extension folder
(`collectors/…`), and `report.path` stays project-relative. Prefer a scaffolded
extension for a code metric: it carries its fixture and page.

Namespace every key (`<extension namespace>.<id>`, or `<vendor>.<id>` in the project's extension: `repo.todo_count`, `acme.bundle_size`).

### `measures:` fields

| field | meaning |
|---|---|
| `key` | namespaced fact-type id (required) |
| `subjectKind` | what each fact is about: `file` \| `symbol` \| `test` \| `model` … |
| `captureScope` | what ONE capture restates: `complete` (default; a whole population, e.g. a report) or `per-path` (only the paths in its snapshot; the fold keeps each path's latest fact, so a snapshot collector's per-file facts stay a repo-wide total while only changed files are rescanned). Set `per-path` on any measure a `snapshot.taken` collector records per-file facts on |
| `unit` | `count` \| `%` \| `ms` \| `kb` \| `usd` … |
| `temporalSemantics` | additivity **over time**: `additive` (SUM across captures — events/tokens), `semi-additive` (take the LAST capture — stocks/counts + level ratios like coverage, default), `non-additive` (re-derive Σnum/Σden across all captures — accumulating ratios) |
| `fold` | how a metric over this measure turns its series into **one number**, in a chart's bucket and in a headline alike: `sum` (events; a ratio's parts re-divide Σn/Σd, Σd = 0 is no data), `last` (a level: the latest capture), `first`, `avg`, or `none` (no single number). A metric over a measure with no `fold`, or `none`, is a `check` error: oxplow never guesses one |

### `metrics:` (spec) fields

| field | meaning |
|---|---|
| `key` / `use` | define a new spec / enable a catalog one (exactly one) |
| `measure` | the measure whose facts this aggregates (an aggregate `expression` needs it; a formula takes none) |
| `expression` | how it reads: an aggregate over a measure's facts **within a capture** — `COUNT(*)`, `SUM(value)`, `AVG(value)`, `MIN(value)`, `MAX(value)`, `LAST(value)` or `RATIO(num, den)` — optionally `FILTER (WHERE …)` joining `value >= n`, `value <= n`, `severity = 's'` and `dim('key') = 's'` with AND (one of each); or a formula over OTHER metrics, `METRIC('a') / METRIC('b')` (`+ - * /`). Anything else is a load error that quotes this grammar |
| `direction` | `higher-better` \| `lower-better` \| `neutral` — red/green sense |
| `target` / `warnAt` / `failAt` | thresholds (via `direction`) — coloring + the Stop-hook nudge |
| `displayKind` | read presentation: `gauge` \| `findings` \| `test` \| `coverage` \| `event` |
| `category` | catalog grouping: `operational` \| `testing` \| `coverage` \| `static-quality` \| `custom` |
| `sliceableDims` | conformed-dimension keys for group-by / drill-across |

**Two-axis aggregation.** `aggregation` combines the facts inside one capture; the
source measure's `fold` governs how the series collapses across
time. A count-over-threshold is `expression: "COUNT(*) FILTER (WHERE value >= N)"`.

### `collectors:` fields (a fact collector)

| field | meaning |
|---|---|
| `id` | namespaced producer id (required) — the producer name its captures carry |
| `doc` | one line on what it collects |
| `runtime` | `starlark` \| `jaq` (sandboxed, no I/O) \| `exec` (a program; needs `process.run`, approved at deploy) |
| `entry` | the script or program, project-relative |
| `trigger` | when it runs: `{ on: [snapshot.taken] }` (after a snapshot that recorded files — tree scans), `{ on: [effort.finished] }` (over the effort's end snapshot), `{ on: [<other event types>], where?: { field: value } }`, `{ every: 15m }`, or `manual` (only via `oxplow.collector.sync`) |
| `facts` | the measure keys it may record facts on (declare-to-collect — a fact outside this list is dropped) |
| `report` | `{ path, format }` — a tool's report file it reads (`text`\|`json`\|`xml`\|`lcov`\|`lines`), parsed into `input.report` |
| `input` | (starlark/jaq) a SQL query whose rows arrive as `input.rows`; binds `:stream_id :snapshot_id :effort_id :thread_id :turn_id`, `:event_id` (the event's id) and `:event_seq` (its seq) |

A collector with `entities:` instead of `facts:` is an **entity collector**
(rows, not measurements) — that belongs in an extension, not the project.

## Step 4 — scope

- **Extension** (the default for a code metric you author): the extension's
  `extension.yaml` declares its own measures, dimensions, collectors and
  metrics (its fact collectors are sandboxed — starlark or jaq).
- **Project**: the project's own extension,
  `.oxplow/extensions/project/extension.yaml` + scripts under its `collectors/`
  (checked into the repo, shared with the team; turn a metric on/off or set
  its target with `use:` / `enabled: false` in `.oxplow/project.yaml`) — for a collector that must be
  an `exec` program under `process.run`, or a report-derived one.
- **User-global** (cross-project): `*.yaml` files under the user's global config
  dir — a `measures/`, `metrics/` and `dimensions/` folder — hot-reloaded.
  Global measures are active in every project automatically; a global *metric*
  is enabled per-project with a `use:`. There are no global collectors.

Precedence is **project > global > built-in** by key; an extension's metrics are
on unless the project mentions the key, or the definition says `default: off`
(then on only once the project `use:`s it). A metric's `collector: <id>` names
the collector in its extension that computes it; that collector runs only while
one of the metrics naming it is on.
