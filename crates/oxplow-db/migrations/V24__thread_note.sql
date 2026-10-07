-- A thread's notes get their own table (they shared `task_note` with the
-- comments on oxplow's tasks, one of `task_id` / `thread_id` set). A note
-- keeps its id; its ref becomes `thread_note:not<n>`, and so do its
-- `page_ref` edges. `task_note` is then only task comments: rebuilt
-- without `thread_id`, its work-item comment triggers (V17) restated.

CREATE TABLE thread_note (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO thread_note (id, thread_id, body, author, created_at)
SELECT id, thread_id, body, author, created_at FROM task_note WHERE thread_id IS NOT NULL;
CREATE INDEX idx_thread_note_thread ON thread_note(thread_id, created_at);

UPDATE page_ref SET source_kind = 'thread_note'
 WHERE source_kind = 'task_note'
   AND source_id IN (SELECT 'not' || id FROM thread_note);
UPDATE page_ref SET target_kind = 'thread_note'
 WHERE target_kind = 'task_note'
   AND target_id IN (SELECT 'not' || id FROM thread_note);

CREATE TABLE task_note_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO task_note_new (id, task_id, body, author, created_at)
SELECT id, task_id, body, author, created_at FROM task_note WHERE task_id IS NOT NULL;
-- Both continue past every id the shared table gave out.
DELETE FROM sqlite_sequence WHERE name IN ('thread_note', 'task_note_new');
INSERT INTO sqlite_sequence (name, seq)
SELECT 'thread_note', seq FROM sqlite_sequence WHERE name = 'task_note'
UNION ALL
SELECT 'task_note_new', seq FROM sqlite_sequence WHERE name = 'task_note';
DROP TABLE task_note;
ALTER TABLE task_note_new RENAME TO task_note;
CREATE INDEX idx_task_note_task ON task_note(task_id, created_at);

CREATE TRIGGER work_item_comment_follows_task_note_insert AFTER INSERT ON task_note
BEGIN
    INSERT INTO work_item_comment (id, ref, body, author, created_at)
    VALUES ('task_note:' || NEW.id, 'work_item:oxplow:tsk' || NEW.task_id, NEW.body, NEW.author,
            NEW.created_at);
END;
CREATE TRIGGER work_item_comment_follows_task_note_update AFTER UPDATE OF body ON task_note
BEGIN
    UPDATE work_item_comment SET body = NEW.body WHERE id = 'task_note:' || NEW.id;
END;
CREATE TRIGGER work_item_comment_follows_task_note_delete AFTER DELETE ON task_note
BEGIN
    DELETE FROM work_item_comment WHERE id = 'task_note:' || OLD.id;
END;
