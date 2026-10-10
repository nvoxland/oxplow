SELECT r.run_id, r.parent_run_id, r.extension, r.contribution, r.kind, r.version,
       r.generation, r.actor_kind, r.actor, r.stream_id, r.thread_id, r.effort_id,
       r.turn_id, r.agent_session_id, r.snapshot_id, r.cause_event_id, r.mode,
       r.started_at, r.finished_at, r.outcome, r.error, r.wall_ms, r.cpu_ms,
       r.peak_bytes, r.ai_calls, r.scopes_json AS scopes, r.decisions_json AS decisions,
       r.started_at = (SELECT max(l.started_at) FROM source('extension_run') l
                        WHERE l.contribution = r.contribution) AS latest
FROM source('extension_run') r
