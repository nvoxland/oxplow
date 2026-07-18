# Dashboards

User-created **dashboards** — a named grid of metric **tiles** the user
assembles to view a handful of metrics at a glance. Complements the per-metric
**detail** page (`.context/metrics.md`) and the **Recorded Metrics** list.
Epic **tsk138**.

**Scope: project-global.** Metrics are project-scoped, so dashboards are too —
one set per project, reachable from any stream/thread. The `dashboard` table has
**no `stream_id`** (the DB is already per-project).

## Data model — `V70__dashboard.sql`

Two tables, mirroring the `comment` / `comment_message` two-table shape:

- **`dashboard`** — `id` (PK AUTOINCREMENT), `title`, `sort_index` (dashboards
  list in a chosen order), `settings_json` (`V71`; the **saved default view** —
  the filter row's range/branch/dimension — opaque for the same reason
  `options_json` is), `created_at` / `updated_at`.
- **`dashboard_item`** — `id`, `dashboard_id` (FK `ON DELETE CASCADE`),
  `sort_index`, `kind` (`metric` | `text`), `metric_key` (null for text tiles),
  `options_json` (**opaque** per-tile blob: viz / mode / scale / size /
  per-tile range+branch override / title override / breakdown dim / text body —
  grows with no migration, exactly like task `payload_json`), `created_at` /
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

**Gotcha:** the `call_mut` closures return `Result<_, DomainError>` (no auto
`From<rusqlite::Error>`), so every `query_row` inside them needs an explicit
`.map_err(map_sql_err)?`.

## Surface — the 11-layer flow

Follows `.context/ipc-and-stores.md`. Cores in
`oxplow-rpc/src/commands/dashboards.rs` (9: `list_dashboards`, `get_dashboard`,
`create_dashboard` → returns the `Dashboard`, `rename_dashboard`,
`delete_dashboard`, `add_dashboard_item` → returns the new `DashboardItemId`,
`update_dashboard_item`, `remove_dashboard_item`, `reorder_dashboard_items`).
**Every write emits `OxplowEvent::DashboardsChanged`** (fieldless — dashboards
are project-global) so agent- and UI-driven edits both live-refresh.

Tauri delegates in `oxplow-tauri-ipc/src/commands/dashboards.rs`; frontend
wrappers + `subscribeDashboardEvents` in `apps/desktop/src/api.ts` (filter
`event.kind === "dashboardsChanged"`).

**Response types serialize snake_case** (`metric_key`, `sort_index`,
`dashboard_id`) — no `rename_all`, matching the codebase's read-type convention
(e.g. `SeriesPoint`). The *request* structs, by contrast, use camelCase
(`#[serde(rename = "metricId")]` etc.). The generated bindings capture both
correctly; frontend field access is snake_case.

## MCP — agent authoring (tsk140)

Reads + create/populate are agent-authorable so the agent can build a dashboard
on request ("make me a dashboard of the coverage metrics"), matching the
`scaffold_metric` direction. In `oxplow-mcp/src/lib.rs`:

- `list_dashboards`, `get_dashboard` — reads.
- `create_dashboard {title}` → returns the new dashboard.
- `add_dashboard_item {dashboard_id, kind, metric_key?, options_json?}` →
  returns the new tile id.

These four are **`both(...)`** in `oxplow-surface-parity/src/lib.rs`; the pure-UI
edits (`rename` / `delete` / `update_item` / `remove_item` / `reorder_items`)
stay **`ui(...)`**. Every MCP write also emits `DashboardsChanged`.

## UI (tsk141 — Phase 3)

Two page kinds, wired per `.context/pages-and-tabs.md`'s "adding a tab kind"
checklist:

- **`custom-dashboard`** — payload-bearing (modeled on `metric-detail`): the id
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
  empty state is a dashed drop-zone card. Live-refreshes on `dashboardsChanged`
  (structure) + `configChanged` (defs).
- **`dashboards`** — a literal-id index kind (**in `INDEX_KINDS`**,
  `dashboardsRef()`): `DashboardsIndexPage` lists the user's dashboards (rows via
  `RouteLink` → `customDashboardRef`) + a **+ New dashboard** action. In the
  launcher via a `computePagesDirectory` **Activity** entry.

**Tiles** — `components/Dashboard/MetricTile.tsx` (+ `TextTile.tsx`). A `metric`
tile switches on the `options_json` `viz`:

| `viz` | Renders |
|---|---|
| `line` (default) | the shared `TrendChart` over the reused `metricDetailData` pipeline, sized near the tile's own width (see below) |
| `number` | a big latest value + a signed delta chip colored by the spec's `direction` |
| `sparkline` | the shared `Sparkline` (lifted out of `RecordedMetricsPage` in tsk142) + the latest value |
| `bar` | the metric rolled up by `dim` (default `package`) via `metricDimensionRollup`, top 6 bars |

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
passes it in; the tile fetches its own samples and refreshes on
`metricSamplesChanged`. Clicking the title drills through to the metric detail;
right-click is the tile menu — **Visualization** and **Size** submenus (checked
= current, writing through `updateDashboardItem`), a **Warn when off target**
toggle, plus open / open-in-new-tab / remove.

**Off-target highlight (tsk149).** With the `alertOffTarget` option on
(**default**), a tile whose latest in-window value misses its target takes the
status color on its card border plus a chip by the title — "Off target" (warn)
or "Failing" (fail); a `number` tile colors its headline too. The wording is
deliberately direction-agnostic: "below target" would be wrong for a
`lower-better` metric, where missing means being *above*. Defaulting on is safe
because it can only fire for a metric that **has** a target, so target-less
metrics never light up.

> The verdict and its color come from **`metricStatus` + `metricStatusColor`**
> in `pages/recordedMetricsRows.ts` — the same classifier behind the Recorded
> Metrics row colors and its Off-target filter, whose doc notes it exists so "a
> color and a filter can't disagree". `metricStatusColor` was **lifted out of**
> `RecordedMetricsPage` (where it was private) rather than copied into the tile.
> Same rule as metric sectioning: one classifier, one color mapping.

**Dimension filter (tsk150)** — a **Filter by** dimension select plus a value
select, scoping the whole dashboard to one slice: *"show me everything for
package `crates/oxplow-app`"*. Tiles keep their own visualization — only the
points feeding them change. This is the same move as the metric-detail page's
breakdown-row click (tsk136), which charts one group's series.

Dimension options are `dashboardBreakoutDims(defs)`: the sorted **union** of what
every tile's metric is sliceable by (per-metric via the canonical
`breakdownDimensions`, lifted out of `MetricDetail.tsx` into
`metricDetailData.ts` so the dashboard and the detail page can't disagree). Value
options are the **union the tiles report upward** (`onGroupValues`) once a
dimension is chosen — the grouped fetch happens as soon as the *dimension* is
picked, because its distinct groups are what populates the value list. Choosing a
dimension alone is not yet a filter; tiles show everything until a value is
picked. The dimension self-clears if the last tile supporting it is removed, and
changing dimension resets the value.

A tile that can't honour the scope keeps showing **all** of its data, struck
through with a **corner-to-corner X** across the whole pane, plus a dashed card
edge and a ⊘ badge whose tooltip explains why and what's shown instead. The X
overlay is `pointerEvents: none` (so it never intercepts the title click or the
right-click menu) and uses `vector-effect: non-scaling-stroke`, since its viewBox
is stretched to a non-square card and the stroke would otherwise render uneven.

Two earlier attempts at this marker are worth not repeating: fading the card to
55% made the chart harder to read for no gain, and printing the reason as an
inline chip tried to fit a package path into chip-sized space. The rule that
emerged: **the chart stays fully legible, the marker carries the meaning, and the
explanation lives in a tooltip** where it has room.

That verdict is the unit-tested `resolveGroupFilter`, and it has **two** causes:
the metric doesn't declare the dimension at all, or it declares it but has no
data under the selected value — a metric can be sliceable by `package` and simply
have no facts in *that* package. While groups are still loading the tile counts
as filtered, so it doesn't flash the unscoped marker on the way to its data.

**Saved view (tsk151)** — a **Save view** button at the end of the filter row
writes the current range/branch/dimension/value to the dashboard's
`settings_json` (`set_dashboard_settings`), and opening the dashboard seeds the
filter row back from it via the pure `parseDashboardSettings`. Only *non-default*
keys are written, so a saved view never pins a filter the user left alone.

Beside it sits **Save Copy** (tsk152): one click calls `duplicate_dashboard`,
which copies every tile in **one transaction** and carries the *current* filter
row as the copy's saved view, names it `<title> (copy)`, and opens it — where
the in-body H1 is already how a dashboard gets renamed. Deliberately two plain
buttons rather than a split control with an inline rename field; that was tried
first and read as fiddly for what is a one-click action. Composing the copy
client-side from `create` + N × `add_item` would be non-atomic and would fire
**N `DashboardsChanged` events**, each re-fetching every open dashboard page;
the store op emits once.

It lives in the **database, not `localStorage`**: dashboards are project-global
and agent-readable over MCP, so a per-machine saved view would be invisible to
both. The seeding is guarded by a `seededFor` ref keyed on the dashboard id —
the same load runs on every `dashboardsChanged` (adding a tile, a rename, an
agent write), and re-seeding there would yank the filters out from under a user
who had since changed them.

**Dashboard filter (tsk142)** — a range + branch control under the header that
**every tile inherits**, via the pure `resolveTileWindow(opts, dashboard, now)`:
a per-tile `range`/`branch` option wins, and a tile `range` of `"all"` explicitly
opts out of a windowed dashboard. Range defaults to **All time** (a dashboard is
an overview; a bounded default would blank out sparse metrics). The branch
options are the **union of the branches the tiles report upward** (`onBranches`),
since no single page-level sample fetch exists. A `bar` tile reads the dimension
roll-up, which is inherently latest-state, so the time filter doesn't apply to it.

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
dashboard the agent creates over MCP appears without a reload. The block hides
for a disabled metric (no spec ⇒ nothing to chart).
