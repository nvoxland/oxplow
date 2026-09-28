-- page_ref kinds become canonical ref kinds (tsk404; .context/refs.md):
-- `task` -> `work_item` (id `oxplow:tsk<n>`), `git-commit` -> `commit`,
-- `directory` -> `dir`, `task-note` -> `task_note`. There is no
-- compatibility layer: the rows are wiped and the boot backfill
-- (page_ref_backfill.rs) regenerates every edge from its source rows
-- under the new vocabulary. v_commit_task is the one view that keyed on
-- the old kinds.
DELETE FROM page_ref;

DROP VIEW v_commit_task;
CREATE VIEW v_commit_task AS
SELECT source_id AS sha,
       CAST(substr(target_id, length('oxplow:tsk') + 1) AS INTEGER) AS task_id
FROM page_ref
WHERE source_kind = 'commit'
  AND target_kind = 'work_item'
  AND target_id LIKE 'oxplow:tsk%';
