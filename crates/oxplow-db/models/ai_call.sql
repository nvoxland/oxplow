SELECT id, role, provider, model, caller, input_tokens, output_tokens,
       latency_ms, ok, error, at
FROM source('ai_call')
