-- What site search indexes for each of the active list's items, through
-- the interface. Archived work leaves search, as its files do (tsk921):
-- an item on an archived thread or stream isn't indexed; the backlog
-- (no thread) is.
SELECT CAST(w.ref AS TEXT) AS ref,
       CAST(w.title AS TEXT) AS title,
       -- Its own id first (`tsk42`, `ENG-12`), so typing it finds it.
       CAST(substr(w.ref, length('work_item:' || w.provider || ':') + 1) || char(10) || w.body
            AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id
FROM ref('work_item') w
LEFT JOIN source('threads') th ON th.id = w.thread_id
LEFT JOIN source('streams') s ON s.id = th.stream_id
WHERE (w.thread_id IS NULL OR th.archived_at IS NULL)
  AND s.archived_at IS NULL
