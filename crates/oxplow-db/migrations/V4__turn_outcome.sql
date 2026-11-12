-- How each turn ended.
--   outcome       completed, interrupted (the person), session_ended (its
--                 harness session ended mid-turn), session_restarted (a new
--                 harness process started with it open), session_closed
--                 (oxplow closed the agent session), restart (oxplow's own
--                 recovery); unknown for an old turn whose end left no
--                 trace. NULL while running. No CHECK: Rust owns the values.
-- What a turn left running is not a column: the Stop's list is on its
-- agent.turn.ended event, and agent_job is the record of each job.
ALTER TABLE agent_turn ADD COLUMN outcome TEXT;

-- Ended turns take the outcome their agent.turn.ended logged (a payload
-- whose details expired has none); the three ends logged `interrupted`,
-- and the turns with no outcome logged, are told apart by the text oxplow
-- wrote into the answer, which then goes: `answer` is only what the agent
-- said. What is left has no trace of how it ended: `unknown`.
UPDATE agent_turn
   SET outcome = (SELECT json_extract(e.payload, '$.outcome') FROM event_log e
                   WHERE e.type = 'agent.turn.ended' AND e.turn_id = agent_turn.id
                     AND e.payload_expired_at IS NULL
                   ORDER BY e.seq DESC LIMIT 1)
 WHERE ended_at IS NOT NULL;
UPDATE agent_turn
   SET outcome = CASE answer
                   WHEN 'session ended' THEN 'session_ended'
                   WHEN 'session restarted' THEN 'session_restarted'
                   WHEN 'session closed' THEN 'session_closed'
                   WHEN 'interrupted_by_restart' THEN 'restart'
                   WHEN 'interrupted' THEN 'interrupted'
                   ELSE outcome END
 WHERE ended_at IS NOT NULL AND (outcome = 'interrupted' OR outcome IS NULL);
UPDATE agent_turn SET answer = NULL
 WHERE (outcome = 'interrupted' AND answer = 'interrupted')
    OR (outcome = 'session_ended' AND answer = 'session ended')
    OR (outcome = 'session_restarted' AND answer = 'session restarted')
    OR (outcome = 'session_closed' AND answer = 'session closed')
    OR (outcome = 'restart' AND answer = 'interrupted_by_restart');
UPDATE agent_turn SET outcome = 'unknown'
 WHERE ended_at IS NOT NULL AND outcome IS NULL;

-- agent.turn.ended@3 → @4: the logged rows take their turn's outcome (core
-- keeps only a type's newest version); an expired payload stays as it is.
UPDATE event_log
   SET payload = json_set(payload, '$.outcome',
                          coalesce((SELECT t.outcome FROM agent_turn t
                                     WHERE t.id = event_log.turn_id AND t.outcome <> 'unknown'),
                                   json_extract(payload, '$.outcome')))
 WHERE type = 'agent.turn.ended' AND v = 3 AND payload_expired_at IS NULL;
UPDATE event_log SET v = 4 WHERE type = 'agent.turn.ended' AND v = 3;
