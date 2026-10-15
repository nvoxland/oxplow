SELECT s.id, s.thread_id, s.kind, s.harness, s.acp_agent, s.title, s.resume_session_id, s.host,
       s.opened_at, s.closed_at, s.closed_reason, s.updated_at, s.pid, s.last_seen_at,
       s.stopped_by_user_at,
       CAST('agent_session:ses' || s.id AS TEXT) AS ref,
       s.native_title, s.fork_next,
       -- What a person calls it: their title, else the one its harness
       -- gives the conversation, else the harness and the day it was
       -- opened. The one place a session is named.
       CAST(coalesce(nullif(trim(s.title), ''), nullif(trim(s.native_title), ''),
                     s.harness || ' ' || substr(s.opened_at, 1, 10)) AS TEXT)
         AS display_name,
       (SELECT b.origin FROM source('agent_session_binding') b
         WHERE b.agent_session_id = s.id AND b.unbound_at IS NULL) AS bound_origin
FROM source('agent_session') s
