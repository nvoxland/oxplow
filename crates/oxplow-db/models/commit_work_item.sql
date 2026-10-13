-- The active work list's items each commit was for.
SELECT DISTINCT c.sha, CAST(w.ref AS TEXT) AS work_item
FROM (
  -- Its message mentions the item. (`page_ref` keys an item by its list's
  -- own `<provider>:<native id>`.)
  SELECT source_id AS sha, target_id AS item
  FROM source('page_ref')
  WHERE source_kind = 'commit'
    AND target_kind = 'work_item'
  UNION
  -- The item declared it as an impact, or its effort's work is in it.
  SELECT target_id, source_id
  FROM source('page_ref')
  WHERE source_kind = 'work_item'
    AND target_kind = 'commit'
) c
JOIN ref('work_item') w ON w.provider || ':' || w.native_id = c.item
