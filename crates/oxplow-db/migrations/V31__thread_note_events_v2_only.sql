-- The thread-note events have one version each: the logged v1 rows (which
-- named a thread's note as a `task_note`, from when the two shared a
-- table) are rewritten to v2's `thread_note`, so v1 and core's last
-- mention of oxplow's task notes go.

UPDATE event_log
   SET v = 2,
       payload = CASE WHEN json_extract(payload, '$.note') LIKE 'task_note:%'
                      THEN json_set(payload, '$.note',
                                    'thread_note:' || substr(json_extract(payload, '$.note'),
                                                             length('task_note:') + 1))
                      ELSE payload END
 WHERE type IN ('knowledge.note.written', 'knowledge.note.deleted') AND v = 1;
