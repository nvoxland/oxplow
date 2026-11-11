-- What site search indexes for each thread: its title, and in the body who
-- did its work (its sessions' names). Closed and archived threads are
-- included: finding last month's work is the point. `version` is the
-- newest of the thread's own change, its sessions' changes and its last
-- turn, so a recompute reads the text of the threads that moved only.
SELECT CAST(t.uri AS TEXT) AS uri,
       CAST(t.title AS TEXT) AS title,
       CAST(trim(
         coalesce((SELECT group_concat(char(10) || s.display_name, '')
                        FROM (SELECT display_name FROM ref('agent_session')
                               WHERE thread_id = t.id ORDER BY opened_at) s), '')
       ) AS TEXT) AS body,
       CAST('str' || t.stream_id AS TEXT) AS stream_id,
       CAST(max(
         t.updated_at,
         coalesce((SELECT max(updated_at) FROM ref('agent_session')
                    WHERE thread_id = t.id), ''),
         coalesce((SELECT max(coalesce(ended_at, started_at)) FROM ref('agent_turn')
                    WHERE thread_id = t.id), '')
       ) AS TEXT) AS version
FROM ref('thread') t
