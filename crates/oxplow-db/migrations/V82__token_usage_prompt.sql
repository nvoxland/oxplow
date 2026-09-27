-- v_token_usage gains the human prompt that opened each turn, so the
-- per-turn token log can be a lens (oxplow-analytics `task-turns`).
DROP VIEW v_token_usage;

CREATE VIEW v_token_usage AS
SELECT id, stream_id, thread_id, effort_id, agent_kind, model, prompt, input_tokens,
       output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, recorded_at
FROM agent_token_usage;
