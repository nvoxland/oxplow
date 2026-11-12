-- When the watcher first and last reported a captured file changed since the
-- stream's previous take. NULL when the take found the change itself (the
-- startup sweep, a file with no watcher event): the change then lies only
-- between the two takes.
ALTER TABLE file_snapshot ADD COLUMN first_changed_at TEXT;
ALTER TABLE file_snapshot ADD COLUMN last_changed_at TEXT;
