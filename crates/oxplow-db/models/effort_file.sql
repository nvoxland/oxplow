SELECT ef.effort_id, e.work_item,
       CASE WHEN e.work_item LIKE 'work_item:oxplow:tsk%'
            THEN CAST(substr(e.work_item, 21) AS INTEGER) END AS task_id,
       ef.path, ef.change_kind, ef.closest_vcs_rev
FROM source('effort_file') ef
JOIN source('effort') e ON e.id = ef.effort_id
