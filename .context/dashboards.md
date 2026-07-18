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
  list in a chosen order), `created_at` / `updated_at`.
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
| `line` (default) | the shared `TrendChart` over the reused `metricDetailData` pipeline |
| `number` | a big latest value + a signed delta chip colored by the spec's `direction` |
| `sparkline` | the shared `Sparkline` (lifted out of `RecordedMetricsPage` in tsk142) + the latest value |
| `bar` | the metric rolled up by `dim` (default `package`) via `metricDimensionRollup`, top 6 bars |

A `text`-kind item is a heading / markdown note (`TextTile`): `InlineEdit`
(multiline) over `MarkdownView`, for grouping a grid.

The page resolves each tile's `def` from one `listMetricDefinitions()` fetch and
passes it in; the tile fetches its own samples and refreshes on
`metricSamplesChanged`. Clicking the title drills through to the metric detail;
right-click is the tile menu — **Visualization** and **Size** submenus (checked
= current, writing through `updateDashboardItem`) plus open / open-in-new-tab /
remove.

**Dashboard filter (tsk142)** — a range + branch control under the header that
**every tile inherits**, via the pure `resolveTileWindow(opts, dashboard, now)`:
a per-tile `range`/`branch` option wins, and a tile `range` of `"all"` explicitly
opts out of a windowed dashboard. Range defaults to **All time** (a dashboard is
an overview; a bounded default would blank out sparse metrics). The branch
options are the **union of the branches the tiles report upward** (`onBranches`),
since no single page-level sample fetch exists. A `bar` tile reads the dimension
roll-up, which is inherently latest-state, so the time filter doesn't apply to it.

**Sizing + reorder (tsk142)** — `tileSpanStyle(size)` maps `wide` →
`gridColumn: span 2` and `tall` → `gridRow: span 2` (the grid sets
`gridAutoRows` so a row span means something). Tiles **drag to reorder** with
MIME `application/x-oxplow-dashboard-tile` (distinct from the rail's section
MIME so a rail drag can't drop into the grid), the pure `moveToIndex` from
`centerTabsReorder.ts`, an inset accent edge as the drop indicator, and a drop on
the grid background meaning "move to the end" → `reorderDashboardItems`.

**Add picker** — `buildAddMetricMenu(catalog, onPick)` (pure) groups
`listMetricCatalog()` into per-category submenus; the page appends an **Add text
/ heading** item. The header button, the empty-state button, and a right-click on
the grid all open it via `useContextMenu`.

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
