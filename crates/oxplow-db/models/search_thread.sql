-- What site search indexes for each thread: its title, and in the body what
-- was done in it (its efforts' titles and summaries) and who did it (its
-- sessions' names). Closed and archived threads are included: finding
-- last month's work is the point. `version` is the newest of the thread's
-- own change, its efforts' starts and ends, its sessions' changes and its
-- last turn (an open effort's summary follows the turns), so a recompute
-- reads the text of the threads that moved only.
SELECT CAST(t.uri AS TEXT) AS uri,
       CAST(t.title AS TEXT) AS title,
       CAST(trim(
         coalesce((SELECT group_concat(char(10) || e.title || char(10) || coalesce(e.summary, ''), '')
                     FROM (SELECT title, summary FROM ref('effort')
                            WHERE thread_id = t.id ORDER BY started_at) e), '')
         || coalesce((SELECT group_concat(char(10) || s.display_name, '')
                        FROM (SELECT display_name FROM ref('agent_session')
                               WHERE thread_id = t.id ORDER BY opened_at) s), '')
       ) AS TEXT) AS body,
       CAST('str' || t.stream_id AS TEXT) AS stream_id,
       CAST(max(
         t.updated_at,
         coalesce((SELECT max(coalesce(ended_at, started_at)) FROM ref('effort')
                    WHERE thread_id = t.id), ''),
         coalesce((SELECT max(updated_at) FROM ref('agent_session')
                    WHERE thread_id = t.id), ''),
         coalesce((SELECT max(coalesce(ended_at, started_at)) FROM ref('agent_turn')
                    WHERE thread_id = t.id), '')
       ) AS TEXT) AS version
FROM ref('thread') t
