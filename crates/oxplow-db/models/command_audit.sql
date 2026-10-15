SELECT id, at, command, actor_kind, actor_id, outcome, error, thread_id, session_id
FROM source('command_audit')
