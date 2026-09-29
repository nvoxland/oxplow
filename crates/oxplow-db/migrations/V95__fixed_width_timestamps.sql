-- tsk387 — every TEXT timestamp column, normalized to the fixed-width form
-- `YYYY-MM-DDTHH:MM:SS.ffffffZ` (27 chars) that `Timestamp` now serializes
-- to by construction (crates/oxplow-domain/src/time.rs).
--
-- Until this version only six stores ran their writes through
-- `canonical_ts`; the other twelve wrote the `time` crate's trimmed RFC 3339
-- (`…20.5Z`, or `…20Z` for a whole second), which sorts AFTER any longer
-- same-prefix neighbour under SQLite's lexicographic comparison — so
-- `ORDER BY created_at` on comment messages, `ORDER BY started_at` on agent
-- turns and every other timestamp sort was wrong whenever two rows landed
-- within the same trimmed prefix (the flaky
-- `thread_grows_and_orders_oldest_first`). The serializer fix stops new rows
-- from being trimmed; this backfills every existing one so old and new rows
-- compare correctly with each other. Same CASE as V67 (tsk107), applied to
-- every `*_at` / `at` TEXT column in the schema at V94 (the list is the
-- schema's, generated from `pragma_table_info`; a test pins that no column is
-- missing). Non-UTC or malformed values (no trailing Z) are left alone.

UPDATE agent_nudge SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE agent_token_cursor SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE agent_token_usage SET recorded_at =
  CASE
    WHEN INSTR(recorded_at, '.') = 0 THEN SUBSTR(recorded_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(recorded_at, 1, INSTR(recorded_at, '.'))
         || SUBSTR(SUBSTR(recorded_at, INSTR(recorded_at, '.') + 1,
                          LENGTH(recorded_at) - INSTR(recorded_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE recorded_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(recorded_at) != 27;

UPDATE agent_tool_call SET at =
  CASE
    WHEN INSTR(at, '.') = 0 THEN SUBSTR(at, 1, 19) || '.000000Z'
    ELSE SUBSTR(at, 1, INSTR(at, '.'))
         || SUBSTR(SUBSTR(at, INSTR(at, '.') + 1,
                          LENGTH(at) - INSTR(at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE at LIKE '____-__-__T__:__:__%Z' AND LENGTH(at) != 27;

UPDATE agent_turn SET ended_at =
  CASE
    WHEN INSTR(ended_at, '.') = 0 THEN SUBSTR(ended_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(ended_at, 1, INSTR(ended_at, '.'))
         || SUBSTR(SUBSTR(ended_at, INSTR(ended_at, '.') + 1,
                          LENGTH(ended_at) - INSTR(ended_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE ended_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(ended_at) != 27;

UPDATE agent_turn SET started_at =
  CASE
    WHEN INSTR(started_at, '.') = 0 THEN SUBSTR(started_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(started_at, 1, INSTR(started_at, '.'))
         || SUBSTR(SUBSTR(started_at, INSTR(started_at, '.') + 1,
                          LENGTH(started_at) - INSTR(started_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE started_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(started_at) != 27;

UPDATE ai_call SET at =
  CASE
    WHEN INSTR(at, '.') = 0 THEN SUBSTR(at, 1, 19) || '.000000Z'
    ELSE SUBSTR(at, 1, INSTR(at, '.'))
         || SUBSTR(SUBSTR(at, INSTR(at, '.') + 1,
                          LENGTH(at) - INSTR(at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE at LIKE '____-__-__T__:__:__%Z' AND LENGTH(at) != 27;

UPDATE change SET computed_at =
  CASE
    WHEN INSTR(computed_at, '.') = 0 THEN SUBSTR(computed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(computed_at, 1, INSTR(computed_at, '.'))
         || SUBSTR(SUBSTR(computed_at, INSTR(computed_at, '.') + 1,
                          LENGTH(computed_at) - INSTR(computed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE computed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(computed_at) != 27;

UPDATE claim SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE code_quality_scan SET ended_at =
  CASE
    WHEN INSTR(ended_at, '.') = 0 THEN SUBSTR(ended_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(ended_at, 1, INSTR(ended_at, '.'))
         || SUBSTR(SUBSTR(ended_at, INSTR(ended_at, '.') + 1,
                          LENGTH(ended_at) - INSTR(ended_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE ended_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(ended_at) != 27;

UPDATE code_quality_scan SET started_at =
  CASE
    WHEN INSTR(started_at, '.') = 0 THEN SUBSTR(started_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(started_at, 1, INSTR(started_at, '.'))
         || SUBSTR(SUBSTR(started_at, INSTR(started_at, '.') + 1,
                          LENGTH(started_at) - INSTR(started_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE started_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(started_at) != 27;

UPDATE command_audit SET at =
  CASE
    WHEN INSTR(at, '.') = 0 THEN SUBSTR(at, 1, 19) || '.000000Z'
    ELSE SUBSTR(at, 1, INSTR(at, '.'))
         || SUBSTR(SUBSTR(at, INSTR(at, '.') + 1,
                          LENGTH(at) - INSTR(at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE at LIKE '____-__-__T__:__:__%Z' AND LENGTH(at) != 27;

UPDATE comment SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE comment SET last_activity_at =
  CASE
    WHEN INSTR(last_activity_at, '.') = 0 THEN SUBSTR(last_activity_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(last_activity_at, 1, INSTR(last_activity_at, '.'))
         || SUBSTR(SUBSTR(last_activity_at, INSTR(last_activity_at, '.') + 1,
                          LENGTH(last_activity_at) - INSTR(last_activity_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE last_activity_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(last_activity_at) != 27;

UPDATE comment SET resolved_at =
  CASE
    WHEN INSTR(resolved_at, '.') = 0 THEN SUBSTR(resolved_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(resolved_at, 1, INSTR(resolved_at, '.'))
         || SUBSTR(SUBSTR(resolved_at, INSTR(resolved_at, '.') + 1,
                          LENGTH(resolved_at) - INSTR(resolved_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE resolved_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(resolved_at) != 27;

UPDATE comment SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE comment_message SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE dashboard SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE dashboard SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE dashboard_item SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE dashboard_item SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE decision SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE effort_acknowledged_path SET acknowledged_at =
  CASE
    WHEN INSTR(acknowledged_at, '.') = 0 THEN SUBSTR(acknowledged_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(acknowledged_at, 1, INSTR(acknowledged_at, '.'))
         || SUBSTR(SUBSTR(acknowledged_at, INSTR(acknowledged_at, '.') + 1,
                          LENGTH(acknowledged_at) - INSTR(acknowledged_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE acknowledged_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(acknowledged_at) != 27;

UPDATE effort_attribution SET recorded_at =
  CASE
    WHEN INSTR(recorded_at, '.') = 0 THEN SUBSTR(recorded_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(recorded_at, 1, INSTR(recorded_at, '.'))
         || SUBSTR(SUBSTR(recorded_at, INSTR(recorded_at, '.') + 1,
                          LENGTH(recorded_at) - INSTR(recorded_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE recorded_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(recorded_at) != 27;

UPDATE effort_metric_delta SET refreshed_at =
  CASE
    WHEN INSTR(refreshed_at, '.') = 0 THEN SUBSTR(refreshed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(refreshed_at, 1, INSTR(refreshed_at, '.'))
         || SUBSTR(SUBSTR(refreshed_at, INSTR(refreshed_at, '.') + 1,
                          LENGTH(refreshed_at) - INSTR(refreshed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE refreshed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(refreshed_at) != 27;

UPDATE effort_observation_row SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE effort_unattributed_file SET recorded_at =
  CASE
    WHEN INSTR(recorded_at, '.') = 0 THEN SUBSTR(recorded_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(recorded_at, 1, INSTR(recorded_at, '.'))
         || SUBSTR(SUBSTR(recorded_at, INSTR(recorded_at, '.') + 1,
                          LENGTH(recorded_at) - INSTR(recorded_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE recorded_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(recorded_at) != 27;

UPDATE event_consumer_checkpoint SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE event_dead_letter SET first_failed_at =
  CASE
    WHEN INSTR(first_failed_at, '.') = 0 THEN SUBSTR(first_failed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(first_failed_at, 1, INSTR(first_failed_at, '.'))
         || SUBSTR(SUBSTR(first_failed_at, INSTR(first_failed_at, '.') + 1,
                          LENGTH(first_failed_at) - INSTR(first_failed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE first_failed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(first_failed_at) != 27;

UPDATE event_dead_letter SET last_failed_at =
  CASE
    WHEN INSTR(last_failed_at, '.') = 0 THEN SUBSTR(last_failed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(last_failed_at, 1, INSTR(last_failed_at, '.'))
         || SUBSTR(SUBSTR(last_failed_at, INSTR(last_failed_at, '.') + 1,
                          LENGTH(last_failed_at) - INSTR(last_failed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE last_failed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(last_failed_at) != 27;

UPDATE event_log SET at =
  CASE
    WHEN INSTR(at, '.') = 0 THEN SUBSTR(at, 1, 19) || '.000000Z'
    ELSE SUBSTR(at, 1, INSTR(at, '.'))
         || SUBSTR(SUBSTR(at, INSTR(at, '.') + 1,
                          LENGTH(at) - INSTR(at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE at LIKE '____-__-__T__:__:__%Z' AND LENGTH(at) != 27;

UPDATE ext_source_state SET last_run_at =
  CASE
    WHEN INSTR(last_run_at, '.') = 0 THEN SUBSTR(last_run_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(last_run_at, 1, INSTR(last_run_at, '.'))
         || SUBSTR(SUBSTR(last_run_at, INSTR(last_run_at, '.') + 1,
                          LENGTH(last_run_at) - INSTR(last_run_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE last_run_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(last_run_at) != 27;

UPDATE file_snapshot SET captured_at =
  CASE
    WHEN INSTR(captured_at, '.') = 0 THEN SUBSTR(captured_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(captured_at, 1, INSTR(captured_at, '.'))
         || SUBSTR(SUBSTR(captured_at, INSTR(captured_at, '.') + 1,
                          LENGTH(captured_at) - INSTR(captured_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE captured_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(captured_at) != 27;

UPDATE git_branch SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE git_commit SET committed_at =
  CASE
    WHEN INSTR(committed_at, '.') = 0 THEN SUBSTR(committed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(committed_at, 1, INSTR(committed_at, '.'))
         || SUBSTR(SUBSTR(committed_at, INSTR(committed_at, '.') + 1,
                          LENGTH(committed_at) - INSTR(committed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE committed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(committed_at) != 27;

UPDATE lsp_diagnostic SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE measure SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE measure SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE metric_capture SET captured_at =
  CASE
    WHEN INSTR(captured_at, '.') = 0 THEN SUBSTR(captured_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(captured_at, 1, INSTR(captured_at, '.'))
         || SUBSTR(SUBSTR(captured_at, INSTR(captured_at, '.') + 1,
                          LENGTH(captured_at) - INSTR(captured_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE captured_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(captured_at) != 27;

UPDATE metric_capture SET ended_at =
  CASE
    WHEN INSTR(ended_at, '.') = 0 THEN SUBSTR(ended_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(ended_at, 1, INSTR(ended_at, '.'))
         || SUBSTR(SUBSTR(ended_at, INSTR(ended_at, '.') + 1,
                          LENGTH(ended_at) - INSTR(ended_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE ended_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(ended_at) != 27;

UPDATE metric_cube_state SET last_captured_at =
  CASE
    WHEN INSTR(last_captured_at, '.') = 0 THEN SUBSTR(last_captured_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(last_captured_at, 1, INSTR(last_captured_at, '.'))
         || SUBSTR(SUBSTR(last_captured_at, INSTR(last_captured_at, '.') + 1,
                          LENGTH(last_captured_at) - INSTR(last_captured_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE last_captured_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(last_captured_at) != 27;

UPDATE metric_spec SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE metric_spec SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE page_visit SET visited_at =
  CASE
    WHEN INSTR(visited_at, '.') = 0 THEN SUBSTR(visited_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(visited_at, 1, INSTR(visited_at, '.'))
         || SUBSTR(SUBSTR(visited_at, INSTR(visited_at, '.') + 1,
                          LENGTH(visited_at) - INSTR(visited_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE visited_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(visited_at) != 27;

UPDATE snapshot SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE streams SET archived_at =
  CASE
    WHEN INSTR(archived_at, '.') = 0 THEN SUBSTR(archived_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(archived_at, 1, INSTR(archived_at, '.'))
         || SUBSTR(SUBSTR(archived_at, INSTR(archived_at, '.') + 1,
                          LENGTH(archived_at) - INSTR(archived_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE archived_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(archived_at) != 27;

UPDATE streams SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE streams SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE task SET completed_at =
  CASE
    WHEN INSTR(completed_at, '.') = 0 THEN SUBSTR(completed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(completed_at, 1, INSTR(completed_at, '.'))
         || SUBSTR(SUBSTR(completed_at, INSTR(completed_at, '.') + 1,
                          LENGTH(completed_at) - INSTR(completed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE completed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(completed_at) != 27;

UPDATE task SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE task SET deleted_at =
  CASE
    WHEN INSTR(deleted_at, '.') = 0 THEN SUBSTR(deleted_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(deleted_at, 1, INSTR(deleted_at, '.'))
         || SUBSTR(SUBSTR(deleted_at, INSTR(deleted_at, '.') + 1,
                          LENGTH(deleted_at) - INSTR(deleted_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE deleted_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(deleted_at) != 27;

UPDATE task SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE task_commit SET recorded_at =
  CASE
    WHEN INSTR(recorded_at, '.') = 0 THEN SUBSTR(recorded_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(recorded_at, 1, INSTR(recorded_at, '.'))
         || SUBSTR(SUBSTR(recorded_at, INSTR(recorded_at, '.') + 1,
                          LENGTH(recorded_at) - INSTR(recorded_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE recorded_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(recorded_at) != 27;

UPDATE task_effort SET ended_at =
  CASE
    WHEN INSTR(ended_at, '.') = 0 THEN SUBSTR(ended_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(ended_at, 1, INSTR(ended_at, '.'))
         || SUBSTR(SUBSTR(ended_at, INSTR(ended_at, '.') + 1,
                          LENGTH(ended_at) - INSTR(ended_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE ended_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(ended_at) != 27;

UPDATE task_effort SET started_at =
  CASE
    WHEN INSTR(started_at, '.') = 0 THEN SUBSTR(started_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(started_at, 1, INSTR(started_at, '.'))
         || SUBSTR(SUBSTR(started_at, INSTR(started_at, '.') + 1,
                          LENGTH(started_at) - INSTR(started_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE started_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(started_at) != 27;

UPDATE task_link SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE task_note SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE threads SET archived_at =
  CASE
    WHEN INSTR(archived_at, '.') = 0 THEN SUBSTR(archived_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(archived_at, 1, INSTR(archived_at, '.'))
         || SUBSTR(SUBSTR(archived_at, INSTR(archived_at, '.') + 1,
                          LENGTH(archived_at) - INSTR(archived_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE archived_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(archived_at) != 27;

UPDATE threads SET closed_at =
  CASE
    WHEN INSTR(closed_at, '.') = 0 THEN SUBSTR(closed_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(closed_at, 1, INSTR(closed_at, '.'))
         || SUBSTR(SUBSTR(closed_at, INSTR(closed_at, '.') + 1,
                          LENGTH(closed_at) - INSTR(closed_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE closed_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(closed_at) != 27;

UPDATE threads SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE threads SET summary_updated_at =
  CASE
    WHEN INSTR(summary_updated_at, '.') = 0 THEN SUBSTR(summary_updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(summary_updated_at, 1, INSTR(summary_updated_at, '.'))
         || SUBSTR(SUBSTR(summary_updated_at, INSTR(summary_updated_at, '.') + 1,
                          LENGTH(summary_updated_at) - INSTR(summary_updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE summary_updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(summary_updated_at) != 27;

UPDATE threads SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE usage_event SET occurred_at =
  CASE
    WHEN INSTR(occurred_at, '.') = 0 THEN SUBSTR(occurred_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(occurred_at, 1, INSTR(occurred_at, '.'))
         || SUBSTR(SUBSTR(occurred_at, INSTR(occurred_at, '.') + 1,
                          LENGTH(occurred_at) - INSTR(occurred_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE occurred_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(occurred_at) != 27;

UPDATE wiki_page SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(created_at) != 27;

UPDATE wiki_page SET updated_at =
  CASE
    WHEN INSTR(updated_at, '.') = 0 THEN SUBSTR(updated_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(updated_at, 1, INSTR(updated_at, '.'))
         || SUBSTR(SUBSTR(updated_at, INSTR(updated_at, '.') + 1,
                          LENGTH(updated_at) - INSTR(updated_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE updated_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(updated_at) != 27;

UPDATE wiki_page_thread_update SET last_seen_at =
  CASE
    WHEN INSTR(last_seen_at, '.') = 0 THEN SUBSTR(last_seen_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(last_seen_at, 1, INSTR(last_seen_at, '.'))
         || SUBSTR(SUBSTR(last_seen_at, INSTR(last_seen_at, '.') + 1,
                          LENGTH(last_seen_at) - INSTR(last_seen_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE last_seen_at LIKE '____-__-__T__:__:__%Z' AND LENGTH(last_seen_at) != 27;
