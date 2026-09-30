-- P5.C1: one table of work items from every provider. The oxplow
-- provider's rows are restated from `task` by the task cores
-- (`task_store::project_work_item_tx`) in the same transaction; an
-- external provider's arrive by projection from its events (P5.C2).
CREATE TABLE work_item (
    ref TEXT PRIMARY KEY,            -- work_item:<provider>:<id>
    provider TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('todo', 'in_progress', 'blocked', 'done', 'canceled')),
    native_state TEXT NOT NULL,
    native TEXT NOT NULL DEFAULT '{}',
    parent_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX idx_work_item_provider ON work_item(provider, deleted_at);
CREATE INDEX idx_work_item_parent ON work_item(parent_ref);

-- A task can vanish without the task cores seeing it: deleting a thread
-- or stream cascades. Its work item goes with it.
CREATE TRIGGER work_item_follows_task_delete AFTER DELETE ON task
BEGIN
    DELETE FROM work_item WHERE ref = 'work_item:oxplow:tsk' || OLD.id;
END;

INSERT INTO work_item (ref, provider, title, body, state, native_state, native,
                       parent_ref, created_at, updated_at, deleted_at)
SELECT 'work_item:oxplow:tsk' || t.id, 'oxplow', t.title, t.description,
       CASE t.status
           WHEN 'ready' THEN 'todo'
           WHEN 'archived' THEN
               CASE WHEN t.completed_at IS NULL THEN 'canceled' ELSE 'done' END
           ELSE t.status
       END,
       t.status,
       json_object('priority', t.priority, 'thread_id', t.thread_id,
                   'sort_index', t.sort_index, 'author', t.author,
                   'completed_at', t.completed_at),
       CASE WHEN t.parent_id IS NULL THEN NULL ELSE 'work_item:oxplow:tsk' || t.parent_id END,
       t.created_at, t.updated_at, t.deleted_at
FROM task t;
