-- Plans the agent put to the person, from `agent.plan.proposed`: Claude's
-- ExitPlanMode and Codex's plan item (opencode's plan_exit has no text, so
-- it logs none). Whether the person accepted one is in no hook, so no
-- column says.
SELECT e.id AS event_id,
       e.seq,
       e.thread_id,
       e.effort_id,
       e.turn_id,
       e.agent_session_id,
       e.at AS proposed_at,
       CAST(json_extract(e.payload, '$.text') AS TEXT) AS text,
       CAST(json_extract(e.payload, '$.path') AS TEXT) AS path
FROM source('event_log') e
WHERE e.type = 'agent.plan.proposed'
