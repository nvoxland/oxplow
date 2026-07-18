-- A dashboard's saved default view: the filter/config line the user chose
-- (time range, branch, filter dimension + value), restored when the dashboard
-- is next opened (tsk151).
--
-- Opaque JSON on purpose, exactly like `dashboard_item.options_json`: the set of
-- things the filter row carries will grow, and a blob lets it grow without a
-- migration. NULL means "no saved view" — the page falls back to its defaults.
ALTER TABLE dashboard ADD COLUMN settings_json TEXT;
