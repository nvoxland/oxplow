-- Efforts are opened and closed by oxplow, not by a task going in
-- progress (.context/work-tracking.md): the work item becomes optional
-- (NULL = unlinked, was ''), at most one effort is open per thread, an
-- effort may carry its own title, and records how it closed.
--
-- Done by column, never by rebuilding the table: about fifteen tables
-- reference effort(id), and a rebuild would cascade their rows away.

-- One open effort per thread: every older open one on a thread closes now.
ALTER TABLE effort ADD COLUMN closed_by TEXT
    CHECK (closed_by IN ('commit', 'switch', 'person', 'agent', 'system'));
UPDATE effort
   SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
       closed_by = 'system'
 WHERE ended_at IS NULL
   AND id NOT IN (SELECT max(id) FROM effort WHERE ended_at IS NULL GROUP BY thread_id);

-- The work item, optional.
DROP INDEX idx_effort_open_unique;
DROP INDEX idx_effort_work_item;
ALTER TABLE effort ADD COLUMN linked_item TEXT;
UPDATE effort SET linked_item = NULLIF(work_item, '');
ALTER TABLE effort DROP COLUMN work_item;
ALTER TABLE effort RENAME COLUMN linked_item TO work_item;
CREATE INDEX idx_effort_work_item ON effort(work_item, started_at DESC)
    WHERE work_item IS NOT NULL;
CREATE UNIQUE INDEX idx_effort_open_per_thread ON effort(thread_id)
    WHERE ended_at IS NULL;

-- A title of its own; NULL means the default (the linked item's title,
-- else the first prompt's first line), derived in v_effort.
ALTER TABLE effort ADD COLUMN title TEXT;
