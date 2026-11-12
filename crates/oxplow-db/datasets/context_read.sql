SELECT id, thread_id, turn_id, path, at
FROM source('agent_tool_call')
WHERE kind = 'read' AND path LIKE '.context/%.md'
