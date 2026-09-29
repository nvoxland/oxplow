SELECT source_id AS sha,
       CAST(substr(target_id, length('oxplow:tsk') + 1) AS INTEGER) AS task_id
FROM source('page_ref')
WHERE source_kind = 'commit'
  AND target_kind = 'work_item'
  AND target_id LIKE 'oxplow:tsk%'
