SELECT id, stream_id, title, status, agent, sort_index, created_at, updated_at,
       closed_at, archived_at, acp_agent
FROM source('threads')
