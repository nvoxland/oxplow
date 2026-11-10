-- Where the agent had trouble in a request, by this review policy's
-- thresholds: a file edited `struggle.repeatedEdits` or more times, or
-- `struggle.failedCommands` or more failed shell commands, across the
-- request's turns.
WITH calls AS (
  SELECT t.request_id, c.thread_id, c.agent_session_id, c.kind, c.path, c.ok
  FROM ref('tool_call') c JOIN ref('agent_turn') t ON t.id = c.turn_id
  WHERE t.request_id IS NOT NULL
)
SELECT request_id, thread_id, agent_session_id, 'repeated_edits' AS kind, path AS subject, count(*) AS count
FROM calls
WHERE kind = 'edit' AND path IS NOT NULL
GROUP BY request_id, thread_id, agent_session_id, path
HAVING count(*) >= config('/struggle/repeatedEdits')
UNION ALL
SELECT request_id, thread_id, agent_session_id, 'failed_commands' AS kind, 'shell' AS subject, count(*) AS count
FROM calls
WHERE kind = 'shell' AND ok = 0
GROUP BY request_id, thread_id, agent_session_id
HAVING count(*) >= config('/struggle/failedCommands')
