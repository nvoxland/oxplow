-- Archived work leaves search (tsk921): a note in an archived thread or
-- stream isn't indexed.
SELECT n.uri AS uri,
       CAST('' AS TEXT) AS title,
       CAST(n.body AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id,
       -- A note is written once.
       CAST(n.created_at AS TEXT) AS version
FROM ref('thread_note') n
LEFT JOIN source('threads') th ON th.id = n.thread_id
LEFT JOIN source('streams') s ON s.id = th.stream_id
WHERE th.archived_at IS NULL
  AND s.archived_at IS NULL
