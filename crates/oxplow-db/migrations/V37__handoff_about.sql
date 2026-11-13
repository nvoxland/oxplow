-- A handoff is about a record of any resource (a task, a page), not only a
-- task: its column is named for that.
ALTER TABLE handoff RENAME COLUMN task TO about;
