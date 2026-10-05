SELECT sha, CAST(task_id AS INTEGER) AS task_id
FROM (
  -- Its message mentions the task.
  SELECT source_id AS sha, substr(target_id, length('oxplow:tsk') + 1) AS task_id
  FROM source('page_ref')
  WHERE source_kind = 'commit'
    AND target_kind = 'work_item'
    AND target_id LIKE 'oxplow:tsk%'
  UNION
  -- The task declared it as an impact, or its effort's work is in it.
  SELECT target_id, substr(source_id, length('oxplow:tsk') + 1)
  FROM source('page_ref')
  WHERE source_kind = 'work_item'
    AND target_kind = 'commit'
    AND source_id LIKE 'oxplow:tsk%'
)
