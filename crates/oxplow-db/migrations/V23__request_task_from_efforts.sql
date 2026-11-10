-- Efforts go away, and with them the record of which task a thread worked
-- on when: carry it over once. For each effort with a task, the requests of
-- its thread's turns that started within the effort's window are linked to
-- the task, basis `assigned` (the thread held the task then). A request
-- already linked on that basis keeps its link.
INSERT OR IGNORE INTO request_task (request_id, task_uri, basis, turn_id, at)
SELECT t.request_id, e.task, 'assigned', t.id, t.started_at
  FROM effort e
  JOIN agent_turn t ON t.thread_id = e.thread_id
 WHERE e.task IS NOT NULL AND t.request_id IS NOT NULL
   AND t.started_at >= e.started_at
   AND (e.ended_at IS NULL OR t.started_at <= e.ended_at);
