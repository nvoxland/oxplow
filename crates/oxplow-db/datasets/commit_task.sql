-- The active task tracker's items each commit was for.
SELECT DISTINCT CAST(c.sha AS TEXT) AS sha, CAST(w.uri AS TEXT) AS task
FROM (
  -- Its message mentions the item.
  SELECT uri_param(source_uri, 'sha') AS sha, target_uri AS item
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/commit'
    AND target_resource = 'oxplow/task'
  UNION
  -- The item declared it as an impact.
  SELECT uri_param(target_uri, 'sha'), source_uri
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/task'
    AND target_resource = 'oxplow/commit'
  UNION
  -- A request that worked on the item (v_request_task) made the commit's
  -- changes (v_commit_request): derived, not stored as its own fact.
  SELECT cr.sha, rt.task_uri
  FROM source('commit_request') cr
  JOIN source('request_task') rt ON rt.request_id = cr.request_id
) c
JOIN ref('task') w ON w.uri = c.item
