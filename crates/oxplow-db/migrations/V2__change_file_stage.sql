-- Change analysis in two stages (tsk1095): the changed files (stage one,
-- cheap, kept current on every move) stamp their own freshness apart from
-- the deep analysis (`computed_at` / `events_to`); a working tree's also
-- records git's in-progress operation and how many files conflict.
ALTER TABLE change ADD COLUMN files_at TEXT;
ALTER TABLE change ADD COLUMN files_events_to INTEGER;
ALTER TABLE change ADD COLUMN conflicted INTEGER;
ALTER TABLE change ADD COLUMN in_progress TEXT;
