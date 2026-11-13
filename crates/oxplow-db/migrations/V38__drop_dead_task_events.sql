-- Core defined eight `task.*` event types (created, state_changed, edited,
-- deleted, linked, unlinked, commented, assigned) and no longer does: a
-- task's change is its extension's own `oxplow.task_record.changed`.
-- `task.recorded` went earlier. No event of these types can be read back
-- now, so the rows go, with what was keyed to them.
CREATE TEMP TABLE v38_event (seq INTEGER PRIMARY KEY);
INSERT INTO v38_event
  SELECT seq FROM event_log
   WHERE type IN ('task.created', 'task.state_changed', 'task.edited',
                  'task.deleted', 'task.linked', 'task.unlinked',
                  'task.commented', 'task.assigned', 'task.recorded');
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v38_event);
DELETE FROM effect_run WHERE event_seq IN (SELECT seq FROM v38_event);
DELETE FROM pending_run WHERE event_seq IN (SELECT seq FROM v38_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v38_event);
DROP TABLE v38_event;
