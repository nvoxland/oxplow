SELECT r.run_id, r.parent_run_id, r.extension, r.contribution, r.kind, r.version,
       r.generation, r.actor_kind, r.actor_id, r.stream_id, r.thread_id,
       r.turn_id, r.session_id, r.snapshot_id, r.cause_event_id, r.mode,
       r.started_at, r.finished_at, r.outcome, r.error, r.wall_ms, r.cpu_ms,
       r.peak_bytes, r.ai_calls, r.scopes_json AS scopes, r.decisions_json AS decisions,
       c.owner, c.id, c.row_counts_json,
       CAST(uri('oxplow/collector', 'collector', c.owner || '/' || c.id) AS TEXT) AS uri,
       CAST(r.run_id IS p.last_run_id AS INTEGER) AS latest
FROM source('collector_run') c JOIN source('contribution_run') r ON r.run_id = c.run_id
LEFT JOIN source('collector_checkpoint') p ON p.owner = c.owner AND p.id = c.id
