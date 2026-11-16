-- Core keeps the newest event version. capability.switched@3 names the
-- local level `local` (it was `personal`): rewrite the recorded switches.
UPDATE event_log
   SET payload = json_set(payload, '$.chosen_by', 'local')
 WHERE type = 'capability.switched' AND v = 2
   AND payload_expired_at IS NULL AND json_extract(payload, '$.chosen_by') = 'personal';
UPDATE event_log SET v = 3 WHERE type = 'capability.switched' AND v = 2;
