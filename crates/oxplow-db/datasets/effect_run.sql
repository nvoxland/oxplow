SELECT r.run_id, r.parent_run_id, r.extension, r.contribution, r.kind, r.version,
       r.generation, r.actor_kind, r.actor_id, r.stream_id, r.thread_id,
       r.turn_id, r.session_id, r.snapshot_id, r.cause_event_id, r.mode,
       r.started_at, r.finished_at, r.outcome, r.error, r.wall_ms, r.cpu_ms,
       r.peak_bytes, r.ai_calls, r.scopes_json AS scopes, r.decisions_json AS decisions,
       e.id, e.effect, e.event_id, e.event_seq, e.attempt, e.origin,
       CAST(NOT EXISTS (
           SELECT 1 FROM source('effect_run') later
           WHERE later.effect = e.effect AND later.event_id = e.event_id
             AND later.attempt > e.attempt
       ) AS INTEGER) AS latest,
       e.command_run_id, e.proposal_id, e.retry_at
FROM source('effect_run') e JOIN source('contribution_run') r ON r.run_id = e.run_id
