# Dashboards, components and artifacts

What to build when the person wants to *see* oxplow's data. Read
`reading-data.md` first: every number on a dashboard is a query, and the
same rules hold.

## Choose the output

| They want | Build | Who sees it, and how fresh |
|---|---|---|
| An answer now ("which tests are flaky?") | `oxplow_show_page { spec }` | Them, as a tab of their thread, live; Save keeps it |
| A page they come back to | a page file (`page-format.md`) | Anyone with the extension, live |
| Several numbers at a glance | a **dashboard** (below) | The project, live |
| A view no viz draws | a **custom component** (below) | As a page: live, sandboxed |
| Something to share outside oxplow (a Claude artifact, a doc) | a **snapshot**: run the queries, put the rows in it (a `rows:` loader, or a markdown table) | Whoever it's shared with; frozen when you made it |

Ask when it isn't clear which. Say which you built and how fresh it is: a
snapshot doesn't update, and its rows are only as complete as the result
you copied (`truncated`, `coverage`). A Claude artifact that reads oxplow's
data live needs a remote oxplow connector, which doesn't exist yet: offer a
snapshot, or a dashboard in oxplow, instead.

## A dashboard is a page

A dashboard is a page whose `view:` is a `grid` of tiles, in the
**Dashboards** launcher category. A tile is a `viz` over a loader or an
`embed` of another page. It lives in an extension of the person's (`my-pages`
by default), so it can be kept, shared and tested like any page.

```yaml page
title: Work at a glance
category: Dashboards
icon: layout-dashboard
answers:
  - { question: "How is work going?" }
loaders:
  open:   { sql: "SELECT count(*) AS n FROM v_work_item WHERE state != 'done'" }
  by_day: { sql: "SELECT date(created_at) AS day, count(*) AS n FROM v_work_item GROUP BY 1 ORDER BY 1" }
view:
  grid:
    - { id: open, title: Open, viz: number, from: open, size: small }
    - { id: trend, title: Created per day, viz: line, from: by_day, size: wide, chart: { x: day, y: n } }
    - { id: tests, embed: { page: oxplow-foundation/problems }, size: wide }
```

- **Start one:** `oxplow.extension.new_dashboard { title, extension? }` writes
  an empty dashboard (an empty grid), deploys it and opens it.
- **Add a tile:** the person's **Pin to Dashboard** (a page's ☰ menu) embeds
  that page, with its params, in the dashboard they pick. You add one by
  editing the page file: an `embed` node, or a `viz` over a new loader.
- **Change one:** reorder by reordering the list, rename by changing
  `title`, remove by deleting the node, resize with `size`. Read the file
  (`oxplow_read { uri: "oxplow:oxplow/page?page=<ext>/<slug>" }`), change the
  YAML, write it back with `oxplow.extension.write_page { extension, page,
  spec, deploy: true }`. The app's own edits do the same.
- Embed an existing page for a tile that is already a page; its params flow
  by name from the dashboard's params and loaders (`view-nodes.md`).
- Metric tiles: a loader over `metric_grid('week')` / `MEASURE('<key>')`
  (`oxplow-metrics`) shown as `number`, `line` or `bar`.
- A `write_page` is refused for a bundled extension, and from a worktree
  stream when the extension's source is in the project (write from the main
  stream). It returns `{ page, path, deployed }`.

Tell the person the title and that it is under Dashboards (Cmd+P).

## A custom component

When no viz draws it (a graph, a timeline, a board of your own), declare a
component and show it with a `component` node (`commands-and-forms.md` has
the manifest keys; `$OXPLOW_BIN extension new component <name>` scaffolds a
working one):

```yaml
# extension.yaml
custom_components:
  - { id: burndown, title: Burndown }
```

```yaml page
# pages/burndown.yaml
title: Burndown
loaders:
  red: { sql: "SELECT bucket, MEASURE('oxplow.tests.red_runs') AS red FROM metric_grid('day') ORDER BY bucket" }
view: { component: { id: burndown } }
```

The component is fed the page's loaders by name and told which changed; it
never fetches for itself. `components/burndown/index.html`:

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
  const draw = (data) => {
    const { columns, rows } = data.red;
    const at = (row, name) => row[columns.indexOf(name)];
    // … draw rows into #chart, reading cells by column name …
  };
  draw(component.data);
  component.onUpdate(({ data }) => draw(data));   // the host re-runs loaders as the data moves
});
```

**Bringing in HTML made elsewhere** (a Claude artifact, a chart you wrote
for a page): it runs in a sandbox, so adapt it first:

- No network: no CDN `<script src="https://…">`, fonts or fetches. Bundle a
  library's file next to `index.html` (256 files, 5 MiB), or draw with SVG.
- No modules (`type="module"`, `import`): plain scripts, in order.
- No storage, cookies or tokens: state lives in the page, or in your
  extension's state through a command.
- Data comes from what oxplow feeds it (`component.data`, `onUpdate`), never
  from rows pasted into the file: pasted rows are a snapshot.
- Colours from the theme (`applyTheme`, the kit's classes), not fixed hex.
- A library: write TypeScript, import it, declare `build: { run: "<the
  project's install and build command>", output: dist, exclude:
  [node_modules] }` with `bundle: dist/<id>` (bundle to one classic script,
  with whatever bundler the project uses, as an IIFE). The person approves the build command at
  deploy (`repair-and-sharing.md`).

A **whole-page component** is a page whose view is one component node; it
fills the page under oxplow's bar. `component.page` has the page's `params`,
`streamId` and `threadId`; `setTitle`, `setActions([...])` / `onAction` (its
buttons go in oxplow's bar), `openForm(command, input)`, `setViewParams`,
`pageMemory` and `showView(yaml)` (hands oxplow an inline page to draw
under it, live) are on `component`. Never use `localStorage` or
`sessionStorage`. Then `oxplow_validate_extension` and `oxplow_run_page`.
