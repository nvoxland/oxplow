-- The task model's items each commit was for.
SELECT DISTINCT CAST(c.sha AS TEXT) AS sha, CAST(w.uri AS TEXT) AS task
FROM (
  -- Its message mentions the item.
  SELECT uri_param(source_uri, 'sha') AS sha, target_uri AS item
  FROM ref('page_ref')
  WHERE source_resource = 'oxplow/commit'
    AND target_resource = 'oxplow/task'
  UNION
  -- The item declared it as an impact.
  SELECT uri_param(target_uri, 'sha'), source_uri
  FROM ref('page_ref')
  WHERE source_resource = 'oxplow/task'
    AND target_resource = 'oxplow/commit'
  UNION
  -- A request that worked on the item (`request_link`) made the commit's
  -- changes (`commit_request`): derived, not stored as its own fact.
  SELECT cr.sha, rt.task
  FROM ref('commit_request') cr
  JOIN ref('request_link') rt ON rt.request = cr.request
) c
JOIN ref('task') w ON w.uri = c.item
