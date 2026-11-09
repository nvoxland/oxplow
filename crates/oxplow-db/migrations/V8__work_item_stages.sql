-- A work list declares its stages, each in one of core's buckets, and an
-- item's record names its stage: core looks the bucket up. `state` stays
-- the bucket; `native_state` is the stage. oxplow's tasks have no
-- `archived` status: an archived task is done when it was completed, else
-- canceled.

ALTER TABLE capability_provider ADD COLUMN stages_json TEXT NOT NULL DEFAULT '[]';

-- The logged records state their stage.
UPDATE event_log
SET payload = json_remove(
      json_set(payload, '$.item.stage',
               CASE json_extract(payload, '$.item.native_state')
                 WHEN 'archived' THEN json_extract(payload, '$.item.state')
                 ELSE json_extract(payload, '$.item.native_state') END),
      '$.item.state', '$.item.native_state')
WHERE type = 'work_item.recorded';

ALTER TABLE work_item RENAME COLUMN native_state TO stage;
UPDATE work_item SET stage = state WHERE provider = 'oxplow' AND stage = 'archived';

-- task: no `archived` status.
UPDATE task SET status = CASE WHEN completed_at IS NOT NULL THEN 'done' ELSE 'canceled' END
WHERE status = 'archived';
CREATE TEMP TABLE task_seq AS SELECT seq FROM sqlite_sequence WHERE name = 'task';
CREATE TABLE task_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER REFERENCES task(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('backlog', 'ready', 'in_progress', 'blocked', 'done', 'canceled')),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'urgent')),
    created_by TEXT NOT NULL CHECK (created_by IN ('user', 'agent', 'system')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    deleted_at TEXT,
    -- Semantic origin (vs. created_by which is the writer).
    author TEXT CHECK (author IN ('user', 'agent')));
INSERT INTO task_new (id, parent_id, title, description, status, priority, created_by,
                      created_at, updated_at, completed_at, deleted_at, author)
SELECT id, parent_id, title, description, status, priority, created_by,
       created_at, updated_at, completed_at, deleted_at, author
FROM task;
DROP TABLE task;
ALTER TABLE task_new RENAME TO task;
UPDATE sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM temp.task_seq), 0))
WHERE name = 'task';
DROP TABLE temp.task_seq;
CREATE INDEX idx_task_parent ON task(parent_id, deleted_at);
