-- P2.5a (tsk427) — `task_effort` becomes `effort`, keyed by a canonical
-- work-item ref instead of a task FK (.context/target-architecture.md
-- §4.3; .context/data-model.md "effort").
--
-- An effort is a bracket of work on a WORK ITEM, which may be an oxplow
-- task (`work_item:oxplow:tsk42`) or another provider's item
-- (`work_item:linear:ENG-12`). The old NOT NULL `task_id` FK tied it to
-- the task table.
--
-- Everything here is in place — RENAME / ADD COLUMN / DROP COLUMN, never
-- DROP TABLE on a parent. Refinery runs this on a `foreign_keys=ON`
-- connection, where dropping `task_effort` would CASCADE through every
-- child (the V18 incident). RENAME rewrites the children's FK clauses to
-- the new name; it does not touch views, so the two effort views are
-- dropped first and rebuilt at the end.
--
-- `task_effort_turn` and `task_commit` are dead leaves (nothing writes or
-- reads them) and go. Losing the task FK loses nothing: tasks are only
-- ever soft-deleted (`deleted_at`), so its CASCADE never fired; efforts
-- still go with their thread.

DROP VIEW v_effort;
DROP VIEW v_effort_file;

DROP TABLE task_effort_turn;
DROP TABLE task_commit;

DROP INDEX idx_task_effort_task;
DROP INDEX idx_task_effort_open_unique;
DROP INDEX idx_task_effort_thread;
DROP INDEX idx_task_effort_file_snapshot;

ALTER TABLE task_effort RENAME TO effort;
ALTER TABLE task_effort_file RENAME TO effort_file;

-- '' is never a valid ref; the store refuses it before writing.
ALTER TABLE effort ADD COLUMN work_item TEXT NOT NULL DEFAULT '';
UPDATE effort SET work_item = 'work_item:oxplow:tsk' || task_id;
ALTER TABLE effort DROP COLUMN task_id;

CREATE INDEX idx_effort_work_item ON effort(work_item, started_at DESC);
CREATE INDEX idx_effort_thread ON effort(thread_id, started_at DESC);
CREATE UNIQUE INDEX idx_effort_open_unique ON effort(work_item) WHERE ended_at IS NULL;
CREATE INDEX idx_effort_file_snapshot ON effort_file(local_snapshot_id);

-- `task_id` is derived for oxplow work items (NULL for other providers),
-- so lenses that join tasks keep working. 21 = length('work_item:oxplow:tsk') + 1.
CREATE VIEW v_effort AS
SELECT e.id, e.work_item,
       CASE WHEN e.work_item LIKE 'work_item:oxplow:tsk%'
            THEN CAST(substr(e.work_item, 21) AS INTEGER) END AS task_id,
       e.thread_id, th.stream_id, e.started_at, e.ended_at,
       e.start_snapshot_id, e.end_snapshot_id, e.summary
FROM effort e
LEFT JOIN threads th ON th.id = e.thread_id;

CREATE VIEW v_effort_file AS
SELECT ef.effort_id, e.work_item,
       CASE WHEN e.work_item LIKE 'work_item:oxplow:tsk%'
            THEN CAST(substr(e.work_item, 21) AS INTEGER) END AS task_id,
       ef.path, ef.change_kind, ef.closest_git_version
FROM effort_file ef
JOIN effort e ON e.id = ef.effort_id;
