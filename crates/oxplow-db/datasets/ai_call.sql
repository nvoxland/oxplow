SELECT id, role, provider, model, caller, input_tokens, output_tokens,
       latency_ms, ok, error, at, input_hash, request_hash, response_hash, run_id,
       CAST(uri('oxplow/ai_call', 'id', id) AS TEXT) AS uri
FROM source('ai_call')
