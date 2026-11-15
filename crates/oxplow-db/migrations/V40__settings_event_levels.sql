-- Core keeps the newest event version. Preserve recorded changes while
-- naming the former personal configuration level local, and allow global
-- choices without changing either published schema at its old version.
UPDATE event_log
   SET payload = json_set(payload, '$.layer', 'local')
 WHERE type = 'config.changed' AND v = 2
   AND payload_expired_at IS NULL AND json_extract(payload, '$.layer') = 'personal';
UPDATE event_log SET v = 3 WHERE type = 'config.changed' AND v = 2;
UPDATE event_log SET v = 2 WHERE type = 'capability.switched' AND v = 1;
