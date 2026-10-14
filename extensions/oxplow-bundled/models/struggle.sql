-- Where the agent had trouble in an effort, by this review policy's
-- thresholds: a file edited 5+ times, or 3+ failed shell commands.
SELECT effort_id, thread_id, agent_session_id, 'repeated_edits' AS kind, path AS subject, count(*) AS count
FROM ref('tool_call')
WHERE kind = 'edit'
  AND path IS NOT NULL AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id, agent_session_id, path
HAVING count(*) >= 5
UNION ALL
SELECT effort_id, thread_id, agent_session_id, 'failed_commands' AS kind, 'shell' AS subject, count(*) AS count
FROM ref('tool_call')
WHERE kind = 'shell' AND ok = 0 AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id, agent_session_id
HAVING count(*) >= 3
