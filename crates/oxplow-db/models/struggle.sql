SELECT effort_id, thread_id, 'repeated_edits' AS kind, path AS subject, count(*) AS count
FROM source('agent_tool_call')
WHERE tool IN ('Edit', 'Write', 'MultiEdit', 'NotebookEdit')
  AND path IS NOT NULL AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id, path
HAVING count(*) >= 5
UNION ALL
SELECT effort_id, thread_id, 'failed_commands' AS kind, 'Bash' AS subject, count(*) AS count
FROM source('agent_tool_call')
WHERE tool = 'Bash' AND ok = 0 AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id
HAVING count(*) >= 3
