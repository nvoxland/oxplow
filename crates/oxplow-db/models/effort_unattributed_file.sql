SELECT u.effort_id, e.work_item,
       CASE WHEN e.work_item LIKE 'work_item:oxplow:tsk%'
            THEN CAST(substr(e.work_item, 21) AS INTEGER) END AS task_id,
       u.path, u.recorded_at
FROM source('effort_unattributed_file') u
JOIN source('effort') e ON e.id = u.effort_id
