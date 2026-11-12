-- What site search indexes for each task, in `v_search_task`'s shape, over
-- the shared task model (TEMPORARY name: the switch renames it
-- `search_task`). A task assigned to a thread is under that thread's
-- stream; any other is project-wide. Archived work leaves search: a task
-- assigned to an archived thread or stream isn't indexed. Its comments
-- follow its own text in the body.
SELECT CAST(w.uri AS TEXT) AS uri,
       CAST(w.title AS TEXT) AS title,
       -- Its own id first (`tsk42`, `ENG-12`), so typing it finds it.
       CAST(w.native_id || char(10) || w.body
            || coalesce((SELECT group_concat(char(10) || c.body, '')
                           FROM (SELECT body FROM ref('task_comment_next')
                                  WHERE uri = w.uri ORDER BY created_at, id) c), '')
            AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id
FROM ref('task_next') w
LEFT JOIN ref('thread') th ON th.uri = w.assignee
LEFT JOIN ref('stream') s ON s.id = th.stream_id
WHERE th.id IS NULL OR (th.archived_at IS NULL AND s.archived_at IS NULL)
