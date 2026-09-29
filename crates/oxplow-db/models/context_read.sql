SELECT id, thread_id, effort_id, path, at
FROM source('agent_tool_call')
WHERE tool = 'Read' AND path LIKE '.context/%.md'
