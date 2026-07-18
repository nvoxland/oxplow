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

## UI

_Phases 3–5 (tsk141–143): the `custom-dashboard` payload-bearing page kind +
`CustomDashboardPage` tile grid, the **New Dashboard** command + dashboards
index, tile types (line / number / sparkline / bar / text), tile sizing,
drag-drop reorder, a dashboard-level time+branch filter tiles inherit, and an
"Add to dashboard…" block on the metric-detail rail. This section is filled in
as those land._
