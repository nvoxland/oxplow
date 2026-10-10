SELECT id, role, provider, model, caller, input_tokens, output_tokens,
       latency_ms, ok, error, at, input_hash, request_hash, response_hash, run_id
FROM source('ai_call')
