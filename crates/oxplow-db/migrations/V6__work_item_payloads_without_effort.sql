-- A task's status no longer opens or closes an effort
-- (.context/work-tracking.md), so `work_item.created` and
-- `work_item.transitioned` lose their `effort` field; the logged payloads
-- drop it too, to read under the narrowed v1 types.
UPDATE event_log
   SET payload = json_remove(payload, '$.effort')
 WHERE type IN ('work_item.created', 'work_item.transitioned')
   AND json_extract(payload, '$.effort') IS NOT NULL;
