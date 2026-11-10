-- The active task tracker's items each commit was for.
SELECT DISTINCT CAST(c.sha AS TEXT) AS sha, CAST(w.uri AS TEXT) AS task
FROM (
  -- Its message mentions the item.
  SELECT uri_param(source_uri, 'sha') AS sha, target_uri AS item
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/commit'
    AND target_resource = 'oxplow/task'
  UNION
  -- The item declared it as an impact, or its effort's work is in it.
  SELECT uri_param(target_uri, 'sha'), source_uri
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/task'
    AND target_resource = 'oxplow/commit'
) c
JOIN ref('task') w ON w.uri = c.item
