SELECT id, thread_id, kind, harness, acp_agent, title, resume_session_id, host,
       opened_at, closed_at, closed_reason, updated_at, pid, last_seen_at, stopped_by_user_at
FROM source('agent_session')
