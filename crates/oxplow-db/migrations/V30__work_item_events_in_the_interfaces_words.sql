-- The work-item events speak the interface's words only: the logged
-- events from before (oxplow's statuses, field names and note refs) are
-- rewritten to the versions every list logs, so their old versions and
-- oxplow's `TaskStatus` leave core (`.context/work-items.md`).

UPDATE event_log
   SET v = 2,
       payload = json_object(
           'work_item', json_extract(payload, '$.work_item'),
           'state', CASE json_extract(payload, '$.status')
                        WHEN 'ready' THEN 'todo'
                        WHEN 'archived' THEN 'canceled'
                        ELSE json_extract(payload, '$.status') END)
 WHERE type = 'work_item.created' AND v = 1;

UPDATE event_log
   SET v = 2,
       payload = json_object(
           'work_item', json_extract(payload, '$.work_item'),
           'fields', (SELECT json_group_array(CASE value
                                                  WHEN 'description' THEN 'body'
                                                  WHEN 'priority' THEN 'native.priority'
                                                  WHEN 'thread' THEN 'list'
                                                  WHEN 'position' THEN 'rank'
                                                  ELSE value END)
                        FROM json_each(event_log.payload, '$.fields')))
 WHERE type = 'work_item.edited' AND v = 1;

UPDATE event_log
   SET v = 2,
       payload = json_object(
           'work_item', json_extract(payload, '$.work_item'),
           'comment', CASE WHEN json_extract(payload, '$.comment') LIKE 'task_note:%'
                           THEN substr(json_extract(payload, '$.comment'),
                                       length('task_note:') + 1)
                           ELSE json_extract(payload, '$.comment') END)
 WHERE type = 'work_item.commented' AND v = 1;

UPDATE event_log SET v = 2
 WHERE type IN ('work_item.linked', 'work_item.deleted', 'work_item.recorded') AND v = 1;

-- A transition logged beside core's `work_item.state_changed` by the same
-- run is that change twice: the duplicate goes (with any dead letter of
-- it). The rest become the `state_changed` they were.
DELETE FROM event_dead_letter
 WHERE event_seq IN (
     SELECT t.seq FROM event_log t
      WHERE t.type = 'work_item.transitioned'
        AND EXISTS (SELECT 1 FROM event_log s
                     WHERE s.type = 'work_item.state_changed' AND s.cause = t.cause
                       AND json_extract(s.payload, '$.work_item')
                           = json_extract(t.payload, '$.work_item')));
DELETE FROM event_log
 WHERE type = 'work_item.transitioned'
   AND EXISTS (SELECT 1 FROM event_log s
                WHERE s.type = 'work_item.state_changed' AND s.cause = event_log.cause
                  AND json_extract(s.payload, '$.work_item')
                      = json_extract(event_log.payload, '$.work_item'));
UPDATE event_log
   SET type = 'work_item.state_changed', v = 1,
       payload = json_object(
           'work_item', json_extract(payload, '$.work_item'),
           'to', CASE json_extract(payload, '$.to')
                     WHEN 'ready' THEN 'todo'
                     WHEN 'archived' THEN
                         CASE json_extract(payload, '$.from')
                             WHEN 'done' THEN 'done' ELSE 'canceled' END
                     ELSE json_extract(payload, '$.to') END)
 WHERE type = 'work_item.transitioned';
