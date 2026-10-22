-- What site search indexes for each agent session: its name, and in the body
-- the harness, the titles of its thread's efforts that overlapped it, and
-- what it touched — the distinct paths of its tool calls and the commands
-- of its shell calls (up to 300 of each). Closed sessions, and sessions on
-- archived threads, are included. `version` is the newest of the session's
-- own change and its last tool call.
SELECT CAST(s.uri AS TEXT) AS uri,
       CAST(s.display_name AS TEXT) AS title,
       CAST(trim(
         s.harness
         || coalesce((SELECT group_concat(char(10) || e.title, '')
                        FROM (SELECT title FROM ref('effort')
                               WHERE thread_id = s.thread_id
                                 AND started_at <= coalesce(s.closed_at, '9999')
                                 AND coalesce(ended_at, '9999') >= s.opened_at
                               ORDER BY started_at) e), '')
         || coalesce((SELECT group_concat(char(10) || p.path, '')
                        FROM (SELECT DISTINCT path FROM ref('tool_call')
                               WHERE agent_session_id = s.id AND path IS NOT NULL AND path <> ''
                               ORDER BY path LIMIT 300) p), '')
         || coalesce((SELECT group_concat(char(10) || c.detail, '')
                        FROM (SELECT DISTINCT detail FROM ref('tool_call')
                               WHERE agent_session_id = s.id AND kind = 'shell'
                                 AND detail IS NOT NULL AND detail <> ''
                               ORDER BY detail LIMIT 300) c), '')
       ) AS TEXT) AS body,
       CAST('str' || th.stream_id AS TEXT) AS stream_id,
       CAST(max(
         s.updated_at,
         coalesce((SELECT max(at) FROM ref('tool_call') WHERE agent_session_id = s.id), '')
       ) AS TEXT) AS version
FROM ref('agent_session') s
LEFT JOIN source('threads') th ON th.id = s.thread_id
