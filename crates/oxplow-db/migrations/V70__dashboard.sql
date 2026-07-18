-- User-created dashboards (epic tsk138) — a grid of metric tiles the user
-- composes. Project-global (the DB is per-project), so no stream_id: a
-- coverage/complexity dashboard is about the project, visible from any worktree.
--
-- Two tables mirroring the comment/comment_message shape. `dashboard_item`
-- carries a `sort_index` for drag-drop reordering (the task reorder pattern:
-- the whole list is rewritten to dense 0..N on each reorder) and an opaque
-- `options_json` blob (viz type, chart mode, scale, size, per-tile range/branch
-- override, title override, text body) so the tile shape can grow with no
-- migration — like task `payload_json`.

CREATE TABLE dashboard (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE dashboard_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dashboard_id INTEGER NOT NULL REFERENCES dashboard(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    -- `metric` (charts one metric) | `text` (a heading / markdown note).
    kind TEXT NOT NULL,
    -- The metric spec key for a `metric` tile; NULL for a `text` tile.
    metric_key TEXT,
    -- Opaque per-tile options JSON (viz/mode/scale/size/overrides/text body).
    options_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_dashboard_item_dashboard_sort ON dashboard_item(dashboard_id, sort_index);
