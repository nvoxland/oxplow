-- The active work list's items only: the interface shows one list at a
-- time, whichever implementation it is (none: nothing).
SELECT w.uri, w.provider,
       CAST(uri_param(w.uri, 'id') AS TEXT) AS native_id,
       w.title, w.body, w.state, w.stage,
       -- How the list names its stage, as it declares it.
       (SELECT CAST(json_extract(st.value, '$.title') AS TEXT)
          FROM json_each(cp.stages_json) st
         WHERE json_extract(st.value, '$.id') = w.stage) AS stage_title,
       w.native,
       -- A deleted parent isn't a live work item: no parent then.
       (SELECT p.uri FROM source('work_item') p
         WHERE p.uri = w.parent_uri AND p.deleted_at IS NULL) AS parent_uri,
       asg.assignee, asg.rank, w.closed_at, w.created_at, w.updated_at
FROM source('work_item') w
-- Who is going to work on it and its place in their queue: core's record,
-- whichever list (none: nobody, ordered by when it was filed).
LEFT JOIN source('work_item_assignment') asg ON asg.uri = w.uri
JOIN source('capability_provider') cp
  ON cp.capability = 'work_items' AND cp.active = 1 AND cp.provider = w.provider
WHERE w.deleted_at IS NULL
