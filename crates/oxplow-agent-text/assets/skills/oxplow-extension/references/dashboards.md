# Dashboards, components and artifacts

What to build when the person wants to *see* oxplow's data, and how. Read
`reading-data.md` first: every number on a dashboard is a query, and the
same rules hold.

## Choose the output

| They want | Build | Who sees it, and how fresh |
|---|---|---|
| An answer now ("which tests are flaky?") | `oxplow_show_lens` with a `spec` | Them, beside the conversation, live; **Keep This** saves it |
| A page they come back to | a lens file (`lens-format.md`) | Anyone with the extension, live |
| Several numbers at a glance | an oxplow **dashboard** (below) | The project, live: tiles re-run as the data moves |
| A view no lens viz draws | a **custom component** (below) | As a lens: live, sandboxed |
| A whole page of your own (a data app: its own layout, actions, forms) | a **custom page**: a component named in `pages:` (below) | A page in the launcher, under oxplow's bar |
| Something to share outside oxplow (a Claude artifact, a doc) | a **snapshot**: run the queries, put the rows in it | Whoever it's shared with; frozen when you made it |

Ask when it isn't clear which. Say which you built and how fresh it is: a
snapshot doesn't update, and its rows are only as complete as the result
you copied (`truncated`, `coverage`). A Claude artifact that reads
oxplow's data live needs a remote oxplow connector, which doesn't exist
yet: offer a snapshot, or a dashboard in oxplow, instead.

## Build a dashboard

A dashboard is a grid of tiles, project-wide. Everything goes through
`mcp__oxplow__run_command`:

1. `oxplow.dashboard.create { title }`: its answer's `id` is the
   dashboard (`dsh3`).
2. `oxplow.dashboard.add_item { dashboard, kind, … }` per tile, in order
   (`position` puts one elsewhere). The tile is checked before it is
   saved: a query that wouldn't run, a display that doesn't exist, or a
   chart naming a column the query doesn't return is refused, with why.
3. Tell them its title; it's under Dashboards (Cmd+P).

Tile kinds:

- **`query`**: `sql` (one statement over the `v_*` models,
  `metric_grid`/`MEASURE` included, as `oxplow_query_sql` takes it) and
  `display`: `table`, `list`, `number`, `markdown`, `bar`, `line`,
  `treemap` or `metric`. A chart names its columns in
  `options_json`'s `chart`: `bar` and `line` an `x` and a `y` (a `series`
  optional), `treemap` a `label` and a `size` (a `group` optional).
  `number` shows the first row's first column.
- **`metric`** display: the metric card (its trend, or the latest value
  with `"viz":"number"`), for one metric spec's key: put the key in
  `options_json` as `metric` and select its series as the example below
  does.
- **`lens`**: `lens_id`, a lens the project has.
- **`text`**: a heading over the tiles after it, `options_json`
  `{"text": "…"}`. It labels; it doesn't contain.

Every tile's `options_json` may set `title` and `size` (`small`, `wide`,
`tall` or `full`). Change one with `oxplow.dashboard.update_item { item,
options_json }`, drop one with `oxplow.dashboard.remove_item { item }`,
reorder with `oxplow.dashboard.reorder_items { dashboard, order }`.
Deleting a dashboard is the person's.

Read what's there before adding to it: `SELECT id, title FROM
v_dashboard`, and `SELECT id, kind, options_json FROM v_dashboard_item
WHERE dashboard_id = …`.

### Example: build health

```json
{ "name": "oxplow.dashboard.create", "input": { "title": "Build health" } }
```

Then, with its id as `dashboard`:

```json
{ "name": "oxplow.dashboard.add_item", "input": { "kind": "text", "options_json": "{\"text\":\"Tests\"}" } }
```

```json
{ "name": "oxplow.dashboard.add_item", "input": { "kind": "query", "display": "number", "sql": "SELECT count(*) AS failing FROM v_test_case_stat WHERE last_status = 'failed'", "options_json": "{\"title\":\"Failing tests\"}" } }
```

```json
{ "name": "oxplow.dashboard.add_item", "input": { "kind": "query", "display": "table", "sql": "SELECT subject, runs, flips FROM v_test_case_stat WHERE flips >= 2 ORDER BY flips DESC", "options_json": "{\"title\":\"Flaky tests\",\"size\":\"wide\"}" } }
```

```json
{ "name": "oxplow.dashboard.add_item", "input": { "kind": "query", "display": "line", "sql": "SELECT bucket, MEASURE('oxplow.coverage.abs_pct') AS coverage_pct FROM metric_grid('week') ORDER BY bucket", "options_json": "{\"title\":\"Coverage by week\",\"chart\":{\"x\":\"bucket\",\"y\":\"coverage_pct\"}}" } }
```

```json
{ "name": "oxplow.dashboard.add_item", "input": { "kind": "query", "display": "metric", "sql": "SELECT g.capture_id, g.bucket AS captured_at, MEASURE('oxplow.coverage.abs_pct') AS value FROM metric_grid('capture') g WHERE MEASURE('oxplow.coverage.abs_pct') IS NOT NULL ORDER BY g.bucket DESC", "options_json": "{\"metric\":\"oxplow.coverage.abs_pct\",\"viz\":\"number\"}" } }
```

A metric with no captures yet still saves; its tile says so until data
arrives.

## A custom component

When no viz draws it (a graph, a timeline, a board of your own), declare
a component and a lens that renders it (`commands-and-forms.md` has the
manifest keys; `oxplow extension new component <name>` scaffolds one that
works):

```yaml
# extension.yaml
custom_components:
  - { id: burndown, title: Burndown }
```

```yaml
# lenses/burndown.yaml
title: Burndown
query: SELECT bucket, MEASURE('oxplow.tests.red_runs') AS red FROM metric_grid('day') ORDER BY bucket
viz: custom
custom: { component: burndown }
```

`components/burndown/index.html`:

```html
<!doctype html>
<meta charset="utf-8">
<link rel="stylesheet" href="/component-lib/oxplow-kit.css">
<div id="chart"></div>
<script src="/component-lib/oxplow-component.js"></script>
<script src="app.js"></script>
```

`components/burndown/app.js`:

```js
oxplow.connect().then((component) => {
  component.applyTheme();
  const draw = (run) => {
    const { columns, rows } = run.result;
    const at = (row, name) => row[columns.indexOf(name)];
    // … draw rows into #chart, reading cells by column name …
  };
  draw(component.run);
  component.onUpdate(draw); // the host re-runs the lens as the data moves
});
```

**Bringing in HTML made elsewhere** (a Claude artifact, a chart you wrote
for a page): it runs in a sandbox, so adapt it before it will:

- No network: no CDN `<script src="https://…">`, fonts or fetches.
  Bundle a library's file next to `index.html` (256 files, 5 MiB), or
  draw with SVG yourself.
- No modules (`type="module"`, `import`): plain scripts, in order.
- No storage, cookies or tokens: state lives in the page, or in your
  extension's state through a command.
- Data comes from the lens's run (`component.run`, `onUpdate`), or from
  another of its `assets` lenses (`component.query`), never from rows
  pasted into the file: pasted rows are a snapshot.
- Colours from the theme (`applyTheme`, the kit's classes), not fixed
  hex, so it reads in light and dark.
- A library (a chart, a timeline): write TypeScript, import it, and
  declare `build: { run: "bun install --frozen-lockfile && bun run build",
  output: dist, exclude: [node_modules] }` with `bundle: dist/<id>`;
  bundle to one classic script (`bun build --format iife`). The deploy
  builds it and the person approves the build command
  (`repair-and-sharing.md`).

Then `oxplow_validate_extension`, and `oxplow_run_lens` on the lens.

## A custom page

When the view is the whole page — its own layout, its own buttons —
name the component in `pages:` instead of a lens:
`pages: [{ id: timeline, title: Timeline, icon: history, category:
Activity, params: [from, to], component: timeline }]`. It fills the page
under oxplow's bar. It has no lens run (`component.run` is null);
`component.page` has the page's `params`, `streamId` and `threadId`.
It reads with `component.querySql(sql, params)` (declare `sql: read` on
the component and `permissions: { sql: read }` on the extension; read
only, `?1` params), titles the page with `setTitle`, puts its actions in
oxplow's bar with `setActions([{ id, label, primary? }])` and hears them
with `onAction`, and opens a declared command's form in oxplow with
`openForm(command, input)`. Don't draw page-level buttons inside the
frame: the bar is oxplow's. `extensions/oxplow-foundation/src/timeline/`
is a working one.
