-- Archived work leaves search (tsk921): a note in an archived thread or
-- stream isn't indexed.
SELECT CAST('note:not' || n.id AS TEXT) AS ref,
       CAST('' AS TEXT) AS title,
       CAST(n.body AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id
FROM source('task_note') n
LEFT JOIN source('threads') th ON th.id = n.thread_id
LEFT JOIN source('streams') s ON s.id = th.stream_id
WHERE n.thread_id IS NOT NULL
  AND th.archived_at IS NULL
  AND s.archived_at IS NULL
