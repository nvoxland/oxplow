SELECT id, role, provider, model, caller, input_tokens, output_tokens,
       latency_ms, ok, error, at, input_hash, request_hash, response_hash, run_id,
       CAST('ai_call:' || id AS TEXT) AS ref
FROM source('ai_call')
