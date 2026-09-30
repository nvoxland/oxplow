SELECT id, input_hash, provider, model, prompt_version, op, role, caller, output_json,
       input_tokens, output_tokens, at, ai_call_id
FROM source('ai_result')
