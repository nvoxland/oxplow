SELECT id, stream_id, thread_id, effort_id, turn_id, agent_kind, model, prompt, input_tokens,
       output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, recorded_at
FROM source('agent_token_usage')
