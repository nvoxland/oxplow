-- Core keeps no tasks: a task is a row of the shared model `oxplow/task_record`
-- (`model__oxplow__task_record`), kept by a store (oxtask has its own file),
-- and what requests worked on is the model `request_task`
-- (`model__oxplow__request_task`). These are the tables core kept before: the
-- tracker's items, links and notes, the projection of every provider's items
-- with their links and comments, who is assigned what, and the request links.
-- Their indexes and change-counter triggers go with them.
DROP TABLE task_assignment;
DROP TABLE task_comment;
DROP TABLE task_link;
DROP TABLE task;
DROP TABLE oxtask_note;
DROP TABLE oxtask_link;
DROP TABLE oxtask;
DROP TABLE request_task;
-- The lookup of a provider item's recorded events by its uri.
DROP INDEX IF EXISTS event_log_task_uri;
DELETE FROM table_generation
 WHERE name IN ('task', 'task_link', 'task_comment', 'task_assignment',
                'oxtask', 'oxtask_link', 'oxtask_note', 'request_task');
