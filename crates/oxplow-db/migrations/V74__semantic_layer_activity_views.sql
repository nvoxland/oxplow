-- Semantic layer: agent-activity views (tsk288). Same contract rules as
-- V73 — read-only, documented in semantic_layer.rs CATALOG, kept in
-- lockstep by schema_docs_match_the_views_exactly.

CREATE VIEW v_effort_file AS
SELECT ef.effort_id, e.task_id, ef.path, ef.change_kind, ef.closest_git_version
FROM task_effort_file ef
JOIN task_effort e ON e.id = ef.effort_id;

CREATE VIEW v_task_note AS
SELECT id, task_id, thread_id, body, author, created_at
FROM task_note;

CREATE VIEW v_task_link AS
SELECT id, thread_id, from_item_id AS from_task_id, to_item_id AS to_task_id,
       link_type, created_at
FROM task_link;

CREATE VIEW v_task_event AS
SELECT id, thread_id, item_id AS task_id, event_type, actor_kind, actor_id,
       payload_json, created_at
FROM task_event;

CREATE VIEW v_agent_turn AS
SELECT id, thread_id, task_id, prompt, answer, started_at, ended_at
FROM agent_turn;

CREATE VIEW v_token_usage AS
SELECT id, stream_id, thread_id, effort_id, agent_kind, model, input_tokens,
       output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, recorded_at
FROM agent_token_usage;

CREATE VIEW v_page_visit AS
SELECT id, thread_id, page_kind, page_id, label, visited_at, duration_ms
FROM page_visit;
