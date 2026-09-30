-- tsk561: before V115, archiving a done task cleared `completed_at`, so
-- V115's backfill (and every restatement since) called it `canceled` —
-- past completed work was undercounted in v_work_item. The event log
-- remembers: a task whose last archive came `from: done` was completed,
-- at its last `done` transition before that archive (or, when the done
-- predates the log, no later than the archive). A task archived before
-- the event log existed has no record and stays `canceled`.
CREATE TEMP TABLE restored_completion AS
WITH archive AS (
    SELECT CAST(substr(json_extract(payload, '$.work_item'), 21) AS INTEGER) AS task_id,
           seq, at, json_extract(payload, '$.from') AS from_state,
           ROW_NUMBER() OVER (PARTITION BY json_extract(payload, '$.work_item')
                              ORDER BY seq DESC) AS nth
    FROM event_log
    WHERE type = 'work_item.transitioned'
      AND json_extract(payload, '$.to') = 'archived'
      AND json_extract(payload, '$.work_item') LIKE 'work_item:oxplow:tsk%'
)
SELECT a.task_id,
       COALESCE((SELECT d.at FROM event_log d
                 WHERE d.type = 'work_item.transitioned'
                   AND json_extract(d.payload, '$.work_item') = 'work_item:oxplow:tsk' || a.task_id
                   AND json_extract(d.payload, '$.to') = 'done'
                   AND d.seq < a.seq
                 ORDER BY d.seq DESC LIMIT 1),
                a.at) AS completed_at
FROM archive a JOIN task t ON t.id = a.task_id
WHERE a.nth = 1 AND a.from_state = 'done'
  AND t.status = 'archived' AND t.completed_at IS NULL;

UPDATE task
SET completed_at = (SELECT r.completed_at FROM restored_completion r WHERE r.task_id = task.id)
WHERE id IN (SELECT task_id FROM restored_completion);

UPDATE work_item
SET state = 'done',
    native = json_set(native, '$.completed_at',
                      (SELECT r.completed_at FROM restored_completion r
                       WHERE 'work_item:oxplow:tsk' || r.task_id = work_item.ref))
WHERE ref IN (SELECT 'work_item:oxplow:tsk' || task_id FROM restored_completion);

DROP TABLE restored_completion;
