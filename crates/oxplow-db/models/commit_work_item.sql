-- The active work list's items each commit was for.
SELECT DISTINCT CAST(c.sha AS TEXT) AS sha, CAST(w.ref AS TEXT) AS work_item
FROM (
  -- Its message mentions the item.
  SELECT uri_param(source_uri, 'sha') AS sha, target_uri AS item
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/commit'
    AND target_resource = 'oxplow/work_item'
  UNION
  -- The item declared it as an impact, or its effort's work is in it.
  SELECT uri_param(target_uri, 'sha'), source_uri
  FROM source('page_ref')
  WHERE source_resource = 'oxplow/work_item'
    AND target_resource = 'oxplow/commit'
) c
JOIN ref('work_item') w ON w.ref = c.item
