SELECT id, thread_id, kind, harness, acp_agent, title, resume_session_id, host,
       opened_at, closed_at, closed_reason, updated_at, pid, last_seen_at, stopped_by_user_at,
       CAST('agent_session:ses' || id AS TEXT) AS ref
FROM source('agent_session')
