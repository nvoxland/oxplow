SELECT e.id, e.task,
       e.thread_id, th.stream_id, e.started_at, e.ended_at,
       e.start_snapshot_id, e.end_snapshot_id,
       -- Its own summary (`oxplow.effort.report`), else the final message of the
       -- last turn in it.
       coalesce(
         nullif(trim(e.summary), ''),
         (SELECT t.answer FROM source('agent_turn') t
           WHERE t.thread_id = e.thread_id AND t.answer IS NOT NULL
             AND t.ended_at >= e.started_at
             AND (e.ended_at IS NULL OR t.started_at <= e.ended_at)
           ORDER BY t.started_at DESC LIMIT 1)
       ) AS summary,
       -- Its own title, else its item's, else the first line of the first
       -- prompt a person typed in it: a turn with an empty prompt (a resume,
       -- another session's message) or a harness's message (its first line
       -- only a tag: `<task-notification>`, `<command-name>/clear</…>`) is
       -- skipped. With none, "Untitled effort".
       coalesce(
         nullif(trim(e.title), ''),
         (SELECT w.title FROM source('task') w WHERE w.uri = e.task),
         (SELECT l.line FROM (
            SELECT p.started_at,
                   trim(CASE WHEN instr(p.text, char(10)) > 0
                             THEN substr(p.text, 1, instr(p.text, char(10)) - 1)
                             ELSE p.text END, ' ' || char(9) || char(13)) AS line
              FROM (SELECT t.started_at,
                           ltrim(t.prompt, ' ' || char(9) || char(10) || char(13)) AS text
                      FROM source('agent_turn') t
                     WHERE t.thread_id = e.thread_id
                       AND (t.ended_at IS NULL OR t.ended_at >= e.started_at)
                       AND (e.ended_at IS NULL OR t.started_at <= e.ended_at)) p) l
           WHERE l.line <> '' AND NOT (l.line LIKE '<%>' AND instr(l.line, ' ') = 0)
           ORDER BY l.started_at LIMIT 1),
         'Untitled effort'
       ) AS title,
       e.closed_by,
       CAST(uri('oxplow/effort', 'id', 'eff' || e.id) AS TEXT) AS uri
FROM source('effort') e
LEFT JOIN source('threads') th ON th.id = e.thread_id
