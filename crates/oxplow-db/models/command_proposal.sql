SELECT id, CAST('proposal:' || id AS TEXT) AS ref, created_at, command, input_json AS input,
       actor_kind, actor_id, thread_id, stream_id, key, preview_json AS preview,
       dry_run_json AS dry_run, decision, decided_at, audit_id, superseded_by
FROM source('command_proposal')
