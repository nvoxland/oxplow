SELECT e.id, e.work_item,
       CASE WHEN e.work_item LIKE 'work_item:oxplow:tsk%'
            THEN CAST(substr(e.work_item, 21) AS INTEGER) END AS task_id,
       e.thread_id, th.stream_id, e.started_at, e.ended_at,
       e.start_snapshot_id, e.end_snapshot_id, e.summary
FROM source('effort') e
LEFT JOIN source('threads') th ON th.id = e.thread_id
