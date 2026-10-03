SELECT CAST('task:tsk' || t.id AS TEXT) AS ref,
       CAST(t.title AS TEXT) AS title,
       CAST('tsk' || t.id || char(10) || t.description AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id
FROM source('task') t
LEFT JOIN source('threads') th ON th.id = t.thread_id
WHERE t.deleted_at IS NULL
