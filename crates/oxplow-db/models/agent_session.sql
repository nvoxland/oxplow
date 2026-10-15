SELECT id, thread_id, kind, harness, acp_agent, title, resume_session_id, host,
       opened_at, closed_at, closed_reason, updated_at, pid, last_seen_at, stopped_by_user_at,
       CAST('agent_session:ses' || id AS TEXT) AS ref,
       -- What a person calls it: its title, else the harness and the day it
       -- was opened. The one place a session is named when it has no title.
       CAST(coalesce(nullif(trim(title), ''), harness || ' ' || substr(opened_at, 1, 10)) AS TEXT)
         AS display_name
FROM source('agent_session')
