# Dashboards

## Page-file dashboards

A dashboard is a page in an extension with `category: Dashboards` and a
`view: { grid: [...] }`. Tiles are nodes, including embeds of other pages;
the page file is the source of their order and settings. This is the
replacement for the older database-backed dashboards described below.

The foundation's `pages/dashboards.yaml` is the **Dashboards** index in
the launcher's **Data** section. It lists `v_page WHERE category =
'Dashboards'`, ordered by title and id, and links each row through its
stored page URI. The index is itself in Data so it never lists itself.
It includes dashboard pages from every enabled extension, rather than
only pages in the default personal extension.

**New dashboard…** is a whole-page form action on
`oxplow.extension.new_dashboard`. The index fixes `extension: my-pages`
and asks for the title; the command derives the page slug, creates an
empty grid, writes the page and deploys it. An existing slug is refused
rather than overwritten. The command's `ui.open_after` opens the returned
page URI. Like `extension.write_page`, authoring is refused for bundled
extensions and for an in-project source from a worktree stream.

Read or edit the YAML for an existing dashboard, then use
`oxplow.extension.write_page { extension, page, spec, deploy: true }` to
save and deploy it. The document text is preserved as supplied; comments
and key order need not be discarded. Visual editing uses the same path:
`pages/dashboardEdits.ts` edits YAML Document nodes, preserving comments,
scalar styles and key order. `DashboardGrid` previews drag order, restores
the server order on failure, and offers tile removal through a right-click
menu and an inline confirmation. `DashboardTitle` saves an inline rename
on Enter; Escape cancels. Bundled or missing sources are read-only.

`page_spec { page }` reads the writable primary source immediately before
an edit, including changes that have not been deployed. Read-only pages
return their deployed YAML. This prevents an edit from rebuilding a page
from its rendered DTO and losing author annotations. The current writer
has no compare-and-swap against concurrent filesystem changes.

The nav bar's **Pin to Dashboard** lists writable dashboard pages and
adds an embed with the page's current params and a stable tile id. Its
**New dashboard…** path uses the remembered `pages.extension` preference,
defaulting to `my-pages`. An inline view is saved as a page before pinning.

## Legacy database-backed dashboards (pending removal)

The desktop no longer draws these: the `custom-dashboard` lens renderer,
the `components/Dashboard/*` tiles, `customDashboardRef` and the
`dashboard.*` API wrappers are gone, so an `oxplow:oxplow/dashboard?id=…`
tab opens on the `resource` fallback. What follows describes the store
and commands still in Rust until they are removed with the lens runtime.

User-created **dashboards** — a named grid of metric **tiles** the user
assembles to view a handful of metrics at a glance. Complements the per-metric
**detail** page (`.context/metrics.md`) and the **Recorded Metrics** list.
Epic **tsk138**.

> **Direction:** a user dashboard is a foundation lens (`dashboard`, the
> renderer of the `oxplow/dashboard` resource): its tiles are the rows of
> a query over `v_dashboard_item` (`tile:`), its editing is lens actions.
> Tiles are `query` (pinned SQL, shown as a lens viz or the metric card),
> `lens` (a lens run with the page's params) or `text` (a heading band).

**Scope: project-global.** Metrics are project-scoped, so dashboards are too —
one set per project, reachable from any stream/thread. The `dashboard` table has
**no `stream_id`** (the DB is already per-project).

## Data model — `V70__dashboard.sql`

Two tables, mirroring the `comment` / `comment_message` two-table shape:

- **`dashboard`** — `id` (PK AUTOINCREMENT), `title`, `sort_index` (dashboards
  list in a chosen order), `settings_json` (`V71`; once the saved default
  view, which is gone — the column stays, unused), `created_at` /
  `updated_at`.
- **`dashboard_item`** — `id`, `dashboard_id` (FK `ON DELETE CASCADE`),
  `sort_index`, `kind` (`query` | `lens` | `text`, `TILE_KINDS`; `add_item`
  refuses anything else), `options_json` (**opaque** per-tile blob: a query
  tile's `sql` / `display` / `metric`, viz / size / per-tile range+branch
  override / title override / text body / lens id — grows with no migration,
  exactly like task `payload_json`), `created_at` /
  `updated_at`. Index `idx_dashboard_item_dashboard_sort(dashboard_id,
  sort_index)`.

Ids surface at the boundary as prefixed strings: **`dsh<id>`** / **`dti<id>`**
(`EntityKind::Dashboard` / `DashboardItem` in `oxplow-domain/src/ids.rs`;
`id_type!(DashboardId …)` / `id_type!(DashboardItemId …)`).

## Store — `oxplow-db/src/dashboard_store.rs`

`SqliteDashboardStore` (async): `list`, `get` (→ `DashboardWithItems` =
dashboard + its tiles in display order), `create`, `rename`, `delete`,
`add_item`, `update_item`, `remove_item`, `reorder_items`. Lists
`ORDER BY sort_index, id`. `create` / `add_item` set `sort_index =
COALESCE(MAX(sort_index), -1) + 1`; `reorder_items` rewrites `0..N` in one
`conn.transaction()` (the task store's reorder pattern). Registered on `Services`
as `dashboard_store` (`oxplow-app/src/lib.rs`).

**Writes are `_tx` cores** (`create_tx`, `rename_tx`, `delete_tx`,
`add_item_tx` — at a `position`, later tiles moving down — `update_item_tx`,
`remove_item_tx`, `reorder_items_tx`) run on a command's connection; the
store's async methods only read (P8.A5).

## Surface — commands and reads

**Every write is a `dashboard.*` command** (`oxplow-app/src/commands/
dashboard.rs`, P8.A5), for the desktop (`runCommand`), an agent
(`run_command`) and a lens alike:

- `Tx`: `oxplow.dashboard.create { title }` (not undoable), `oxplow.dashboard.rename
  { dashboard, title }`, `oxplow.dashboard.remove_item { item }` (undone by adding
  it back at its position), `oxplow.dashboard.reorder_items { dashboard, order }`,
  `oxplow.dashboard.delete { dashboard }` (a person's or a lens's,
  `Confirm::Destructive`; the page's Delete action asks first; not
  undoable).
- `Tx` with a `Precheck`: `oxplow.dashboard.add_item { dashboard, kind, sql?,
  display?, lens_id?, title?, size?, text?, metric?, viz?, chart?,
  options_json?, position? }` and `oxplow.dashboard.update_item { item,
  options_json?, sql?, display?, lens_id?, title?, size?, text?, metric?,
  viz?, chart? }`. The flat fields are the tile's option keys (`size` in
  `SIZES`, `viz` in `METRIC_VIZ`, `chart` an object; checked in
  `build_tile`) so a lens form can edit one at a time; on add a flat field
  beats the same key in `options_json`. An update **merges**: it starts
  from `options_json` if given (which alone replaces the whole blob, as
  agents rely on) else the stored options, and overlays the flat fields
  (`merged_update`; a stored retired `size`/`viz` such as `sparkline`
  reads as absent and is dropped, so editing another field of an old tile
  isn't refused). A tile is checked as `dashboard_tiles::new_tile`
  does, an edited one as a new tile of its kind (its options parse, a
  query tile keeps its SQL — `SqlGateway::check`: the read contract,
  `MEASURE()` resolved — and a display it can show, a lens tile its lens).
  The SQL check is async, so it is the command's `Precheck`, run before
  the transaction (a refusal is invalid at `""` / `/options_json`, and
  `/item` for an unknown tile); the handler inside rebuilds the tile with
  the sync `build_tile` and writes over the `_tx` cores. So they join a
  composite's transaction and undo with it. Both undo
  (remove the tile; restore the old options).

Dashboards and tiles are named by id (`dsh3`, `dti7`). The reads stay
IPC core in `oxplow-rpc/src/commands/dashboards.rs` (`list_dashboards`;
the dashboard page reads `v_dashboard` / `v_dashboard_item` itself); views re-read when `ModelsChanged` names `v_dashboard`
or `v_dashboard_item` (`subscribeDashboardEvents` in
`apps/desktop/src/api.ts`) — whoever wrote it.

**Response types serialize snake_case** (`sort_index`,
`dashboard_id`) — no `rename_all`, matching the codebase's read-type convention
(e.g. `SeriesPoint`). The *request* structs, by contrast, use camelCase
(`#[serde(rename = "metricId")]` etc.). The generated bindings capture both
correctly; frontend field access is snake_case.

## Agent authoring (tsk140)

An agent builds a dashboard on request ("make me a dashboard of the
coverage metrics") with `run_command oxplow.dashboard.create` then
`oxplow.dashboard.add_item` (P8.A5: the `create_dashboard` / `add_dashboard_item`
MCP tools are gone). It reads with `oxplow_query_sql` over `v_dashboard` and
`v_dashboard_item`. A query tile's `sql` is checked before it is stored, so a tile
that saves runs; deleting a dashboard is a person's.

What an agent is taught is the `oxplow-extension` skill's `dashboards`
reference (`crates/oxplow-agent-text/assets/skills/oxplow-extension/references/dashboards.md`):
choosing between `oxplow_show_lens`, a lens, a dashboard, a custom
component and a snapshot for outside oxplow (no live connector exists
yet); every tile kind with its options; and adapting HTML made elsewhere
into a sandboxed component. Its examples run in
`crates/oxplow-app/src/dashboards_reference_tests.rs`, read out of the
reference file, so the text can't drift from what the commands accept.

## UI

Two pages, both foundation lenses:

- **`custom-dashboard`** — the renderer of `oxplow/dashboard`
  (`lens: dashboard`), a `document` of two bare parts:
  - `dashboard-title` (a `detail` lens: the dashboard's title and its
    actions — **Add Metric…**, a form whose one field chooses from
    `dashboard-metrics` (`v_metric_catalog` by category) and runs
    `add_item` with just the metric key; **Add Heading…** and
    **Rename…**, one-field forms that open as prompt strips; **Delete**,
    asks first and closes the page, `then: close`). A dashboard that is
    gone says so there (`empty`).
  - `dashboard-tiles` (a `dashboard` lens with `tile:`): the rows of
    `v_dashboard_item` through `json_extract` of the options. Drag runs
    `reorder_items` with `{{order.id}}`; a heading's `edit: true` action
    saves its `text`; right-click gives Rename…, Chart as Line / Number
    (a metric card only, `when: is_metric`), Size: Small / Wide / Tall /
    Full and Remove (Delete key). With no tiles it says `empty`.
  The page's filters are `range` (a `date_range`, default All time — a
  dashboard is an overview; a bounded default would blank out sparse
  metrics) and `branch`, which window every metric card (the loader
  counts them as read by a tile dashboard). The page's `id` is the URI's
  (`uri_param(:uri, 'id')`, the document's row), so it isn't a filter.
  Metric cards, query tiles and heading bands are the props-driven
  `MetricTileBody`, `QueryTileBody` and `TextBand`
  (`components/Dashboard/`), drawn by `DashboardViz` in a kit
  `TileGrid` / `TileSlot` (`src/kit/Tiles.tsx`); a lens tile is run as a
  part. The tab's title is "Dashboard" (a label can't name a row's
  column).
- **`dashboards`** — the foundation's `dashboards` lens page
  (the `dashboards` page, `dashboardsRef()`): the user's
  dashboards over `v_dashboard` (each row links to its page) + a **New
  Dashboard** action on `oxplow.dashboard.create`, whose `ui.open_after`
  opens the new one. In the launcher under **Data**.

**Query tiles (P4.7, tsk492).** A dashboard tile is `query` (pinned SQL
plus how to show it), `lens` or `text`; the `metric` kind and its
`metric_key` column are gone. `V108__dashboard_query_tiles.sql` rewrote each
metric tile into a query tile over `metric_grid('capture')` with `display:
"metric"` and `metric: <key>`, keeping its other options. `display` is a
lens visualization (`QueryTileBody`, the explorer's "Pin to dashboard",
`PinToDashboard.tsx`) or `metric`, the metric card below; `add_item` with `display: metric` and the metric's key needs no `sql`:
the tile pins the metric's captures (`metric_series_sql`, the SQL the UI's
`metricSeriesSql` builds). `metricTile(key)` in `customDashboardData.ts`
builds the same tile for the metric page's "Add to dashboard".
`update_item` re-checks a changed `sql`.

**Chart tiles (P6.F1).** A `bar`, `line` or `treemap` query tile carries
its columns in the options' `chart` — `bar`/`line` `{ x, y, series? }`,
`treemap` `{ label, size, group? }`, the roles a lens's `chart:` names.
`new_tile` refuses a chart display without its required roles, or one
naming a column the query doesn't return (it dry-runs the SQL for one
row). The tile's `chart` column reaches `QueryTileBody`, which passes it to
`adHocLens`, so a pinned chart renders as it did in Explore Data.

**Metric card** — `MetricTileBody` in `components/Dashboard/MetricTile.tsx` (+ `TextTile.tsx`).
A `display: "metric"` query tile runs its `sql` through `oxplow_query_sql` and
switches on the `options_json` `viz`:

| `viz` | Renders |
|---|---|
| `line` (default) | the shared `TrendChart` over the reused `metricDetailData` pipeline, sized near the tile's own width (see below) |
| `number` | a big latest value + a signed delta chip colored by the spec's `direction` |

A line tile charts the metric the way it rolls up (`defaultChartMode`: sum →
cumulative, avg → moving average, else the value). **Simplified (tsk309):**
the `sparkline` / `bar` visualizations, per-tile chart mode and scale, the
off-target highlight, the dimension filter, saved views and Save Copy are
gone; tiles saved with those options read as plain line tiles. Breakdowns are for agents (`metric_grid(…, dim)` through `oxplow_query_sql`)
and lenses.

A `text`-kind item is a **heading band** (`TextBand`) labelling the run of tiles
beneath it: **plain text**, edited in place with `InlineEdit`, rendered as one
left-aligned heading. It defaults to the `full` size, sizes to its own text
height, and wears only a bottom rule — **not a card**.

> It is deliberately **not** markdown. Rendering it through `MarkdownView`
> pulled in the `.oxplow-md` class, which self-caps at `78ch` with
> `margin-inline: auto` outside a reading column — so the heading **centred
> itself** in the band — and let a stray `##` restyle the whole row. A label
> above a group of tiles doesn't need a document renderer (tsk147). It first shipped as a `wide` card inside a 260px-minimum grid row,
which turned a one-line heading into a big empty panel and implied it
*contained* the tiles after it. **It does not**: grouping is positional only.
True tile-owning sections (membership on `dashboard_item`, collapse, drag-in)
were considered and declined as beyond the "just a grid that flows" scope
(tsk147). New text tiles seed **empty** so the placeholder invites a real title.

The grid reads the metric definitions once (`listMetricDefinitions()`) and
passes each card its `def`; a card runs its own query and re-runs through
`useRerunOnChange` when what it read changed. Clicking a card's title drills
through to the metric detail.

**Tile legibility (tsk144).** `TrendChart` renders through a viewBox, so its
coordinate-space size *is* its text scale: the original 760×220 chart squeezed
into a 320px tile shrank its 9px tick labels ~2.4× into illegibility. Tiles
therefore pass explicit `width`/`height` (400×200, or 820/380 for wide/tall) so
the drawing sits near 1:1, and the chart goes `compact` below 520px — tighter
gutters and only two time ticks, since date labels are wide. The grid's minimum
track is **400px** (not 320) for the same reason, with `gridAutoRows: minmax(260px,
auto)`; compact visualizations center themselves so a tall row isn't half empty.

**Watch the `fmtTick` shadowing trap.** `MetricDetail.tsx` has a module-level
`fmtTick` (epoch → date label). A local range-adaptive **y**-tick formatter once
took the same name and shadowed it, so the x axis and hover tooltip silently
rendered raw epoch milliseconds. It is now `fmtYTick`, and
`pages/TrendChart.test.tsx` renders the SVG and asserts no axis label is a bare
10+-digit integer — a pure-helper test can't catch a "wrong function called in
JSX" bug.

**Sizing + reorder.** A tile's `size` (`small`, `wide` = 2 columns,
`tall` = 2 rows, `full` = the grid's width) is a column of the tile query.
The grid's `gridAutoRows` is **`auto`**, not a fixed minimum, so a heading
band can be one line tall; a metric card asserts its own `minHeight`
instead (240, or 500 for `tall`). Tiles drag like any lens row
(`LENS_ROW_DRAG_MIME`): a drop beside a tile or on the grid puts the
dragged one there and runs the drag action with every tile in the new
order.

**Pin to Dashboard → New Dashboard…** asks for the new dashboard's name
inline (Enter creates it and pins, Escape cancels; tsk1045 — it used to
make "My Dashboard" without asking).

**New Dashboard command** — `oxplow.dashboard.create`'s `ui` in
`oxplow-foundation` (Tasks group): it creates "Untitled dashboard", then
opens it (`open_after: oxplow:oxplow/dashboard?id={{result.id}}`), no
form.

Pure helpers live in `pages/customDashboardData.ts` (React-free, unit-tested):
`metricTile`, `latestValue`, `deltaTone`, `buildAddToDashboardMenu`.

**Add to dashboard** — the Metric page's header has an **Add to Dashboard…** action: a form over `oxplow.dashboard.add_item` that chooses from the dashboards (`metric-dashboards`) and adds a metric card (the line viz) for the page's metric; the server builds the card's SQL from the key. It does not make a new dashboard (the Dashboards page does). Pin to Dashboard on other pages is unchanged.
