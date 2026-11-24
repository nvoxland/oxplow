SELECT r.run_id, r.parent_run_id, r.extension, r.contribution, r.kind, r.version,
       r.generation, r.actor_kind, r.actor_id, r.stream_id, r.thread_id,
       r.turn_id, r.session_id, r.snapshot_id, r.cause_event_id, r.mode,
       r.started_at, r.finished_at, r.outcome, r.error, r.wall_ms, r.cpu_ms,
       r.peak_bytes, r.ai_calls, r.scopes_json AS scopes, r.decisions_json AS decisions,
       a.id, a.command,
       CAST(coalesce(r.outcome = 'ok' AND a.input_json IS NOT NULL, 0) AS INTEGER) AS changed,
       CAST(coalesce(a.undone_by, 0) > 0 AS INTEGER) AS undone,
       CAST(uri('oxplow/contribution_run', 'id', r.run_id) AS TEXT) AS uri
FROM source('command_run') a JOIN source('contribution_run') r ON r.run_id = a.run_id
