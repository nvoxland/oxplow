-- The tasks of every store, in today's `v_task` shape, over the shared
-- model (TEMPORARY name: the switch renames it `task`). `native` is the
-- store's own fields; assignee and rank are oxplow's. Fields are read by
-- joining the field table (its key and value index), never through the
-- model's per-row `fields` JSON, which this query leaves unread.
SELECT r.uri, r.store AS provider, r.id AS native_id, r.title, r.body, r.state,
       r.stage, r.stage_title,
       (SELECT CAST(json_group_object(n.name, json(n.value)) AS TEXT)
          FROM ref('task_record_field') n
         WHERE n.uri = r.uri AND n.namespace = r.store) AS native,
       -- A deleted parent isn't a live task: no parent then.
       (SELECT p.uri FROM ref('task_record') p WHERE p.uri = r.parent) AS parent_uri,
       CAST(json_extract(a.value, '$') AS TEXT) AS assignee,
       CAST(json_extract(k.value, '$') AS REAL) AS rank,
       r.closed_at, r.created_at, r.updated_at
FROM ref('task_record') r
LEFT JOIN ref('task_record_field') a ON a.uri = r.uri
                             AND a.namespace = 'oxplow' AND a.name = 'assignee'
LEFT JOIN ref('task_record_field') k ON k.uri = r.uri
                             AND k.namespace = 'oxplow' AND k.name = 'rank'
