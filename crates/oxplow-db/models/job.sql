SELECT j.id,
       j.contribution_id AS job,
       j.intent_key,
       CAST(substr(j.contribution_id, 1, instr(j.contribution_id, '/') - 1) AS TEXT) AS extension,
       j.package_version AS version,
       j.contract_version,
       j.operation,
       j.state,
       j.attempt,
       CAST(j.cancellation_requested_at_ms IS NOT NULL AS INTEGER) AS cancellation_requested,
       j.invoker,
       j.run_id,
       CAST(strftime('%Y-%m-%dT%H:%M:%fZ', j.created_at_ms / 1000.0, 'unixepoch') AS TEXT) AS created_at,
       CAST(strftime('%Y-%m-%dT%H:%M:%fZ', j.updated_at_ms / 1000.0, 'unixepoch') AS TEXT) AS updated_at,
       CAST((SELECT count(*) FROM source('extension_job_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS ai_calls,
       CAST((SELECT count(*) FROM source('extension_job_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'tool_call') AS INTEGER) AS tool_calls,
       CAST((SELECT COALESCE(sum(json_extract(o.content_json, '$.inputTokens')), 0)
               FROM source('extension_job_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS input_tokens,
       CAST((SELECT COALESCE(sum(json_extract(o.content_json, '$.outputTokens')), 0)
               FROM source('extension_job_output') o
              WHERE o.job_id = j.id AND o.attempt = j.attempt AND o.kind = 'usage') AS INTEGER) AS output_tokens
FROM source('extension_job') j
