-- tsk107 — normalize this store's TEXT timestamps to the canonical
-- fixed-width form (6-digit fraction, `…T10:00:20.500000Z`, 27 chars).
--
-- `analytics_stores`' writes never adopted `canonical_ts` (fact_store did, see
-- database.rs), so these columns mix fraction widths and a lexicographic
-- ORDER BY can invert same-second neighbors ("…20.5Z" > "…20.51Z"). That
-- ordering is load-bearing: `latest_snapshot_id_for_stream` picks the row the
-- git-refs re-stamp targets, and the metric-ancestry resolver's stamps are
-- consumed oldest-first. Writes are canonical from this version on; this
-- backfills what already exists.
--
-- The CASE arms: already-canonical rows (27 chars) pass through; no fraction
-- ⇒ append ".000000"; short fraction ⇒ right-pad with zeros to 6. Non-UTC or
-- malformed values (no trailing Z) are left untouched rather than guessed at.

UPDATE snapshot SET created_at =
  CASE
    WHEN INSTR(created_at, '.') = 0 THEN SUBSTR(created_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(created_at, 1, INSTR(created_at, '.'))
         || SUBSTR(SUBSTR(created_at, INSTR(created_at, '.') + 1,
                          LENGTH(created_at) - INSTR(created_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE created_at LIKE '%Z' AND LENGTH(created_at) != 27;

UPDATE file_snapshot SET captured_at =
  CASE
    WHEN INSTR(captured_at, '.') = 0 THEN SUBSTR(captured_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(captured_at, 1, INSTR(captured_at, '.'))
         || SUBSTR(SUBSTR(captured_at, INSTR(captured_at, '.') + 1,
                          LENGTH(captured_at) - INSTR(captured_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE captured_at LIKE '%Z' AND LENGTH(captured_at) != 27;

UPDATE page_visit SET visited_at =
  CASE
    WHEN INSTR(visited_at, '.') = 0 THEN SUBSTR(visited_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(visited_at, 1, INSTR(visited_at, '.'))
         || SUBSTR(SUBSTR(visited_at, INSTR(visited_at, '.') + 1,
                          LENGTH(visited_at) - INSTR(visited_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE visited_at LIKE '%Z' AND LENGTH(visited_at) != 27;

UPDATE usage_event SET occurred_at =
  CASE
    WHEN INSTR(occurred_at, '.') = 0 THEN SUBSTR(occurred_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(occurred_at, 1, INSTR(occurred_at, '.'))
         || SUBSTR(SUBSTR(occurred_at, INSTR(occurred_at, '.') + 1,
                          LENGTH(occurred_at) - INSTR(occurred_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE occurred_at LIKE '%Z' AND LENGTH(occurred_at) != 27;

UPDATE code_quality_scan SET started_at =
  CASE
    WHEN INSTR(started_at, '.') = 0 THEN SUBSTR(started_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(started_at, 1, INSTR(started_at, '.'))
         || SUBSTR(SUBSTR(started_at, INSTR(started_at, '.') + 1,
                          LENGTH(started_at) - INSTR(started_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE started_at LIKE '%Z' AND LENGTH(started_at) != 27;

UPDATE code_quality_scan SET ended_at =
  CASE
    WHEN INSTR(ended_at, '.') = 0 THEN SUBSTR(ended_at, 1, 19) || '.000000Z'
    ELSE SUBSTR(ended_at, 1, INSTR(ended_at, '.'))
         || SUBSTR(SUBSTR(ended_at, INSTR(ended_at, '.') + 1,
                          LENGTH(ended_at) - INSTR(ended_at, '.') - 1) || '000000', 1, 6)
         || 'Z'
  END
 WHERE ended_at IS NOT NULL AND ended_at LIKE '%Z' AND LENGTH(ended_at) != 27;
