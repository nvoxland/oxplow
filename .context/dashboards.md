# Dashboards

User-created **dashboards** — a named grid of metric **tiles** the user
assembles to view a handful of metrics at a glance. Complements the per-metric
**detail** page (`.context/metrics.md`) and the **Recorded Metrics** list.
Epic **tsk138**.

> **Direction (epic tsk275, revised 2026-09-27):** custom dashboards
> **stay in core** as the simple starting point, alongside the Metrics
> pages and Explore Data. Tiles can now be `metric`, `text` or **`lens`**.
> A `lens` tile keeps its `lensId` in `options_json`,
> `parseTileOptions` passes it through, and it renders via `LensTile`.
> See [extensions.md](./extensions.md) → "Core explorer".

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
`conn.transaction()` (the `task_store` reorder pattern). Registered on `Services`
as `dashboard_store` (`oxplow-app/src/lib.rs`).

**Writes are `_tx` cores** (`create_tx`, `rename_tx`, `delete_tx`,
`add_item_tx` — at a `position`, later tiles moving down — `update_item_tx`,
`remove_item_tx`, `reorder_items_tx`) run on a command's connection; the
store's async methods only read (P8.A5).

## Surface — commands and reads

**Every write is a `dashboard.*` command** (`oxplow-app/src/commands/
dashboard.rs`, P8.A5), for the desktop (`runCommand`), an agent
(`run_command`) and a lens alike:

- `Tx`: `dashboard.create { title }` (not undoable), `dashboard.rename
  { dashboard, title }`, `dashboard.remove_item { item }` (undone by adding
  it back at its position), `dashboard.reorder_items { dashboard, order }`,
  `dashboard.delete { dashboard }` (a person's or a lens's,
  `Confirm::Destructive`; the page's Delete `InlineConfirm` is the
  confirmation, so the desktop runs it confirmed; not undoable).
- `External`: `dashboard.add_item { dashboard, kind, sql?, display?,
  lens_id?, options_json?, position? }` and `dashboard.update_item { item,
  options_json? }` — a query tile's SQL is checked by the semantic engine
  first (`dashboard_tiles::new_tile` / `SqlGateway::check`: the read
  contract, `MEASURE()` resolved), which is async and can't run inside the
  bus's transaction; the write that follows is one statement. Both undo
  (remove the tile; restore the old options).

Dashboards and tiles are named by id (`dsh3`, `dti7`). The reads stay
IPC cores in `oxplow-rpc/src/commands/dashboards.rs` (`list_dashboards`,
`get_dashboard`); views re-read when `ModelsChanged` names `v_dashboard`
or `v_dashboard_item` (`subscribeDashboardEvents` in
`apps/desktop/src/api.ts`) — whoever wrote it.

**Response types serialize snake_case** (`sort_index`,
`dashboard_id`) — no `rename_all`, matching the codebase's read-type convention
(e.g. `SeriesPoint`). The *request* structs, by contrast, use camelCase
(`#[serde(rename = "metricId")]` etc.). The generated bindings capture both
correctly; frontend field access is snake_case.

## Agent authoring (tsk140)

An agent builds a dashboard on request ("make me a dashboard of the
coverage metrics") with `run_command dashboard.create` then
`dashboard.add_item` (P8.A5: the `create_dashboard` / `add_dashboard_item`
MCP tools are gone). It reads with the `list_dashboards` / `get_dashboard`
MCP tools. A query tile's `sql` is checked before it is stored, so a tile
that saves runs; deleting a dashboard is a person's.

## UI (tsk141 — Phase 3)

Two page kinds, wired per `.context/pages-and-tabs.md`'s "adding a tab kind"
checklist:

- **`custom-dashboard`** — payload-bearing (modeled on `metric`): the id
  is `custom-dashboard:dsh<n>`, `customDashboardRef(id)` carries the id in both
  the tab id and payload, and a **`refFromTabId` case** rebuilds it from a
  history-restored tab (no payload). `CustomDashboardPage` uses `Page`
  **`layout="full"`** + `titleInBody` and owns its own padding — a tile grid
  wants every pixel, so it deliberately does **not** use the details layout
  (whose 78ch reading column + 320px rail squeeze the grid; the rail version
  was reverted after review). The page renders its own **header row** instead:
  the editable title `<h1>` (`InlineEdit` → `renameDashboard`) on the left,
  **+ Add metric** and a **Delete** `InlineConfirm` on the right. Body is a
  responsive flow grid
  (`grid-template-columns: repeat(auto-fill, minmax(320px, 1fr))`) of tiles;
  empty state is a dashed drop-zone card. Live-refreshes when `v_dashboard`
  / `v_dashboard_item` change (structure); the metric specs and catalog re-read when their models change
  (`useRerunOnChange`).
- **`dashboards`** — a literal-id index kind (**in `INDEX_KINDS`**,
  `dashboardsRef()`): `DashboardsIndexPage` lists the user's dashboards (rows via
  `RouteLink` → `customDashboardRef`) + a **+ New dashboard** action. In the
  launcher via a `computePagesDirectory` **Activity** entry.

**Query tiles (P4.7, tsk492).** A dashboard tile is `query` (pinned SQL
plus how to show it), `lens` or `text`; the `metric` kind and its
`metric_key` column are gone. `V108__dashboard_query_tiles.sql` rewrote each
metric tile into a query tile over `metric_grid('capture')` with `display:
"metric"` and `metric: <key>`, keeping its other options. `display` is a
lens visualization (`QueryTile.tsx`, the explorer's "Pin to dashboard",
`PinToDashboard.tsx`) or `metric`, the metric card below; `metricTile(key)`
in `customDashboardData.ts` builds that tile for the picker and the metric
page. `update_dashboard_item {id, optionsJson}` re-checks a changed `sql`.

**Chart tiles (P6.F1).** A `bar`, `line` or `treemap` query tile carries
its columns in the options' `chart` — `bar`/`line` `{ x, y, series? }`,
`treemap` `{ label, size, group? }`, the roles a lens's `chart:` names.
`new_tile` refuses a chart display without its required roles, or one
naming a column the query doesn't return (it dry-runs the SQL for one
row). `parseTileOptions` keeps `chart` and `QueryTile` passes it to
`adHocLens`, so a pinned chart renders as it did in Explore Data.

**Metric card** — `components/Dashboard/MetricTile.tsx` (+ `TextTile.tsx`).
A `display: "metric"` query tile runs its `sql` through `query_sql` and
switches on the `options_json` `viz`:

| `viz` | Renders |
|---|---|
| `line` (default) | the shared `TrendChart` over the reused `metricDetailData` pipeline, sized near the tile's own width (see below) |
| `number` | a big latest value + a signed delta chip colored by the spec's `direction` |

A line tile charts the metric the way it rolls up (`defaultChartMode`: sum →
cumulative, avg → moving average, else the value). **Simplified (tsk309):**
the `sparkline` / `bar` visualizations, per-tile chart mode and scale, the
off-target highlight, the dimension filter, saved views and Save Copy are
gone; `parseTileOptions` reads tiles saved with those options as plain line
tiles. Breakdowns are for agents (`metric_grid(…, dim)` through `query_sql`)
and lenses.

A `text`-kind item is a **heading band** (`TextTile`) labelling the run of tiles
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

The page resolves each tile's `def` from one `listMetricDefinitions()` fetch and
passes it in; the tile runs its own query and re-runs through
`useRerunOnChange` when what it read changed — its metric's measures, or a
model the query read. Clicking the title drills through to the metric detail;
right-click is the tile menu — **Visualization** and **Size** submenus (checked
= current, writing through `updateDashboardItem`), plus open /
open-in-new-tab / remove.

**Dashboard filter (tsk142)** — a range + branch control under the header that
**every tile inherits**, via the pure `resolveTileWindow(opts, dashboard, now)`:
a per-tile `range`/`branch` option wins, and a tile `range` of `"all"` explicitly
opts out of a windowed dashboard. Range defaults to **All time** (a dashboard is
an overview; a bounded default would blank out sparse metrics). The branch
options are the **union of the branches the tiles report upward** (`onBranches`),
since no single page-level sample fetch exists.

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

**Sizing + reorder (tsk142)** — `tileSpanStyle(size)` maps `full` →
`gridColumn: 1 / -1`, `wide` → `gridColumn: span 2`, `tall` → `gridRow: span 2`.
The grid's `gridAutoRows` is **`auto`**, not a fixed minimum, so a heading band
can be one line tall; a metric tile asserts its own `minHeight` on the card
instead (240, or 500 for `tall`) rather than leaning on the row track (tsk147).
Tiles **drag to reorder** with
MIME `application/x-oxplow-dashboard-tile` (distinct from the rail's section
MIME so a rail drag can't drop into the grid), the pure `moveToIndex` from
`centerTabsReorder.ts`, and a drop on the grid background meaning "move to the
end" → `reorderDashboardItems`.

The **drop indicator** is an absolutely-positioned bar in the grid gap
(`dashboard-drop-line`), on the side the drop will land — which half of the
hovered tile the pointer is in decides before/after. Two traps it was written
around (tsk148): an **inset `box-shadow` on the tile wrapper is invisible**,
because the opaque `TileCard` fills the wrapper and paints over it (that was the
first attempt, and it never showed); and the drop handler **recomputes the side
from its own event** instead of reading the `overSide` state, since the final
`dragOver`'s `setState` may not have re-rendered before `drop` fires.

**Add picker (tsk145)** — `MetricPickerPanel`, an anchored popover: a focused
search box over a scrollable, categorized list, opened by the header button, the
empty-state button, or a right-click on the grid. ↑/↓ walk the flattened rows,
Enter adds, Escape / click-away closes. **Clicking a metric adds it and leaves
the panel open** (rows already on the dashboard show ✓) — assembling a dashboard
means adding several tiles, so the panel is a workbench, not a one-shot menu. A
footer action adds a text/heading tile.

> **Metric sectioning has exactly one home.** The picker's sections come from
> `pickerSections` (`components/Dashboard/metricPicker.ts`), which delegates to
> **`buildMetricSections`** in `pages/metricCategories.ts` — the same rule the
> Recorded Metrics page uses, including its split of `static-quality` into
> per-language sections. The first cut of this picker carried its *own* category
> table and consequently grouped metrics differently from the rest of the app
> (wrong labels, no Coverage group, one giant "Static quality" bucket that
> overflowed the screen). That is precisely the drift `buildMetricSections`'s doc
> comment warns about — **do not reintroduce a local category table.** Search is
> the launcher's `fuzzyMatches` over title + key.

Two traps the sticky section headers hit, worth knowing before styling any
sticky header: a sticky element does **not** automatically paint above later
siblings, so without an explicit `z-index` the row buttons (which follow it in
DOM order) render on top and the header collides with rows scrolling under it;
and `opacity` on the header dims its **background** too, letting rows show
through a supposedly opaque bar — dim the text `color` instead.

**New Dashboard command** — `dashboard.new` in `commands.ts` (Tasks/"plan"
group); the App handler create-then-navigates (`createDashboard` →
`customDashboardRef`), no form.

Pure helpers live in `pages/customDashboardData.ts` (React-free, unit-tested):
`parseTileOptions` (tolerant of null/malformed JSON, drops unknown enum values),
`latestValue`, `deltaTone`, `buildAddMetricMenu`, `tileSpanStyle`,
`resolveTileWindow`.

**Add to dashboard (tsk143)** — the metric-detail page's Details rail carries a
**Dashboard** block: an "Add to dashboard ▾" button whose menu is the pure
`buildAddToDashboardMenu(dashboards, onPick, onNew)` — one entry per dashboard,
then a separator and **New dashboard…**. The new tile inherits the chart
currently on screen (`mode` + `scale`), so it captures the view you were looking
at rather than a default. Picking an **existing** dashboard keeps you on the
metric and shows an **undo toast** (undo removes the tile just added); **New
dashboard…** creates, adds, and navigates to it (a brand-new dashboard is worth
showing). The picker list is kept live via `subscribeDashboardEvents`, so a
dashboard the agent creates appears without a reload. The block hides
for a disabled metric (no spec ⇒ nothing to chart).
