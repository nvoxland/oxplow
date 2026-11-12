-- The tasks of every store, in today's `v_task` shape, over the shared
-- model (TEMPORARY name: the switch renames it `task`). `native` is the
-- store's own fields; assignee and rank are oxplow's.
SELECT r.uri, r.store AS provider, r.id AS native_id, r.title, r.body, r.state,
       r.stage, r.stage_title,
       CAST(json_extract(r.fields, '$.' || r.store) AS TEXT) AS native,
       -- A deleted parent isn't a live task: no parent then.
       (SELECT p.uri FROM ref('task_record') p WHERE p.uri = r.parent) AS parent_uri,
       CAST(json_extract(r.fields, '$.oxplow.assignee') AS TEXT) AS assignee,
       CAST(json_extract(r.fields, '$.oxplow.rank') AS REAL) AS rank,
       r.closed_at, r.created_at, r.updated_at
FROM ref('task_record') r
