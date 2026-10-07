-- The active work list's items each commit was for.
SELECT DISTINCT c.sha, CAST(w.ref AS TEXT) AS work_item
FROM (
  -- Its message mentions the item.
  SELECT source_id AS sha, 'work_item:' || target_id AS work_item
  FROM source('page_ref')
  WHERE source_kind = 'commit'
    AND target_kind = 'work_item'
  UNION
  -- The item declared it as an impact, or its effort's work is in it.
  SELECT target_id, 'work_item:' || source_id
  FROM source('page_ref')
  WHERE source_kind = 'work_item'
    AND target_kind = 'commit'
) c
JOIN ref('work_item') w ON w.ref = c.work_item
