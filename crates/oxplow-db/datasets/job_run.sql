SELECT r.run_id, r.parent_run_id,
       CAST(CASE WHEN j.attempt = 0 THEN d.extension ELSE r.extension END AS TEXT) AS extension,
       r.contribution, r.kind,
       CAST(CASE WHEN j.attempt = 0 THEN j.package_version ELSE r.version END AS TEXT) AS version,
       r.generation, r.actor_kind, r.actor_id, r.stream_id, r.thread_id,
       r.turn_id, r.session_id, r.snapshot_id, r.cause_event_id, r.mode,
       r.started_at, r.finished_at, r.outcome, r.error, r.wall_ms, r.cpu_ms,
       r.peak_bytes, r.ai_calls, r.scopes_json AS scopes, r.decisions_json AS decisions,
       j.id,
       j.contribution_id AS job,
       j.intent_key,
       j.contract_version,
       j.operation,
       j.state,
       j.attempt,
       CAST(j.cancellation_requested_at_ms IS NOT NULL AS INTEGER) AS cancellation_requested,
       CAST(strftime('%Y-%m-%dT%H:%M:%fZ', j.created_at_ms / 1000.0, 'unixepoch') AS TEXT) AS queued_at,
       CAST(uri('oxplow/job_run', 'id', j.id) AS TEXT) AS uri,
       CAST((SELECT count(*) FROM source('job_run_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS rounds,
       CAST((SELECT count(*) FROM source('job_run_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'tool_call') AS INTEGER) AS tool_calls,
       CAST((SELECT COALESCE(sum(json_extract(o.content_json, '$.inputTokens')), 0)
               FROM source('job_run_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS input_tokens,
       CAST((SELECT COALESCE(sum(json_extract(o.content_json, '$.outputTokens')), 0)
               FROM source('job_run_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS output_tokens
FROM source('job_run') j
LEFT JOIN source('job_run_attempt') a ON a.job_id = j.id AND a.attempt = j.attempt
LEFT JOIN source('contribution_run') r ON r.run_id = a.run_id
LEFT JOIN ref('contribution') d ON d.uri = j.contribution_id AND j.attempt = 0
