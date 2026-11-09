-- Who is going to work on an item is core's record, for every work list,
-- not the list's: an item is assigned to a thread, to the person, or to
-- nobody, and has a place in that assignee's queue. A thread no longer
-- owns an item's list, and the backlog is a state (`backlog`: filed, not
-- ready), not "no thread". A closed item has no assignee: who worked on
-- it is its efforts.
CREATE TABLE work_item_assignment (
    uri         TEXT PRIMARY KEY REFERENCES work_item(uri) ON DELETE CASCADE,
    -- `oxplow:oxplow/thread?id=thr3`, `oxplow:oxplow/person`; NULL: nobody.
    assignee    TEXT,
    -- Its place in the assignee's queue (ascending).
    rank        REAL NOT NULL,
    assigned_at TEXT NOT NULL
) STRICT;
CREATE INDEX idx_work_item_assignment_assignee ON work_item_assignment(assignee, rank);

-- An open item on a thread's list is assigned to that thread, in its
-- order there; an open one on no thread keeps its order among the
-- unassigned.
INSERT INTO work_item_assignment (uri, assignee, rank, assigned_at)
SELECT uri,
       CASE WHEN thread_id IS NULL THEN NULL ELSE 'oxplow:oxplow/thread?id=thr' || thread_id END,
       coalesce(rank, 0), updated_at
FROM work_item
WHERE deleted_at IS NULL AND state NOT IN ('done', 'canceled')
  AND (thread_id IS NOT NULL OR rank IS NOT NULL);

-- oxplow's ready tasks on no thread were the backlog: they are now in the
-- backlog state.
UPDATE task SET status = 'backlog'
WHERE thread_id IS NULL AND status = 'ready' AND deleted_at IS NULL;
UPDATE work_item SET state = 'backlog', native_state = 'backlog'
WHERE provider = 'oxplow' AND thread_id IS NULL AND state = 'todo';

-- A list's logged records no longer state the list or rank they were on.
UPDATE event_log SET payload = json_remove(payload, '$.item.list', '$.item.rank')
WHERE type = 'work_item.recorded';

-- work_item: the backlog state; no thread or rank (the assignment's now).
CREATE TABLE work_item_new (
    uri TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL
        CHECK (state IN ('backlog', 'todo', 'in_progress', 'blocked', 'done', 'canceled')),
    native_state TEXT NOT NULL,
    native TEXT NOT NULL DEFAULT '{}',
    parent_uri TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    closed_at TEXT
);
INSERT INTO work_item_new (uri, provider, title, body, state, native_state, native,
                           parent_uri, created_at, updated_at, deleted_at, closed_at)
SELECT uri, provider, title, body, state, native_state, native,
       parent_uri, created_at, updated_at, deleted_at, closed_at
FROM work_item;
DROP TABLE work_item;
ALTER TABLE work_item_new RENAME TO work_item;
CREATE INDEX idx_work_item_provider ON work_item(provider, deleted_at);
CREATE INDEX idx_work_item_parent ON work_item(parent_uri);

-- task: the backlog status; no thread or order (core's assignment).
CREATE TEMP TABLE task_seq AS SELECT seq FROM sqlite_sequence WHERE name = 'task';
CREATE TABLE task_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER REFERENCES task(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('backlog', 'ready', 'in_progress', 'blocked', 'done', 'canceled', 'archived')),
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
