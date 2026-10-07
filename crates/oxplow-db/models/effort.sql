SELECT e.id, e.work_item,
       e.thread_id, th.stream_id, e.started_at, e.ended_at,
       e.start_snapshot_id, e.end_snapshot_id,
       -- Its own summary (`effort.report`), else the final message of the
       -- last turn in it.
       coalesce(
         nullif(trim(e.summary), ''),
         (SELECT t.answer FROM source('agent_turn') t
           WHERE t.thread_id = e.thread_id AND t.answer IS NOT NULL
             AND t.ended_at >= e.started_at
             AND (e.ended_at IS NULL OR t.started_at <= e.ended_at)
           ORDER BY t.started_at DESC LIMIT 1)
       ) AS summary,
       -- Its own title, else its item's, else the first line of the
       -- prompt of the thread's first turn in it.
       coalesce(
         nullif(trim(e.title), ''),
         (SELECT w.title FROM source('work_item') w WHERE w.ref = e.work_item),
         (SELECT trim(CASE WHEN instr(t.prompt, char(10)) > 0
                           THEN substr(t.prompt, 1, instr(t.prompt, char(10)) - 1)
                           ELSE t.prompt END)
            FROM source('agent_turn') t
           WHERE t.thread_id = e.thread_id
             AND (t.ended_at IS NULL OR t.ended_at >= e.started_at)
             AND (e.ended_at IS NULL OR t.started_at <= e.ended_at)
           ORDER BY t.started_at LIMIT 1)
       ) AS title,
       e.closed_by
FROM source('effort') e
LEFT JOIN source('threads') th ON th.id = e.thread_id
