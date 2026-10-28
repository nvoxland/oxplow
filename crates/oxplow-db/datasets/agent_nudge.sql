SELECT id, thread_id, effort_id, turn_id, kind, message, trigger, created_at, delivered_at, audience
FROM source('agent_nudge')
