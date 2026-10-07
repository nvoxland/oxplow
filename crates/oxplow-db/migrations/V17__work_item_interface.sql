-- The work-item interface carries what any list's screens need — the list
-- an item is on, its rank in it, when it closed — and its links and
-- comments, so nothing outside a work list's implementation reads that
-- list's own tables (`.context/work-items.md`). oxplow's tasks fill it in
-- the task's transaction (`task_store::project_work_item_tx`) and by the
-- triggers below; another provider by `work_item.recorded`.

ALTER TABLE work_item ADD COLUMN thread_id INTEGER;
ALTER TABLE work_item ADD COLUMN rank REAL;
ALTER TABLE work_item ADD COLUMN closed_at TEXT;

UPDATE work_item
   SET thread_id = CASE WHEN provider = 'oxplow'
                        THEN CAST(json_extract(native, '$.thread_id') AS INTEGER)
                        ELSE filed_in_thread END;
UPDATE work_item SET rank = json_extract(native, '$.sort_index') WHERE provider = 'oxplow';
UPDATE work_item
   SET closed_at = coalesce(json_extract(native, '$.completed_at'), updated_at)
 WHERE state IN ('done', 'canceled');

ALTER TABLE work_item DROP COLUMN filed_in_thread;

CREATE TABLE work_item_link (
    from_ref TEXT NOT NULL,
    to_ref TEXT NOT NULL,
    link_type TEXT NOT NULL CHECK (link_type IN
        ('blocks', 'relates_to', 'discovered_from', 'duplicates', 'supersedes', 'replies_to')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_ref, to_ref, link_type)
);

CREATE TABLE work_item_comment (
    -- The provider's own id for it (oxplow: `task_note:<id>`).
    id TEXT PRIMARY KEY,
    ref TEXT NOT NULL,
    body TEXT NOT NULL,
    author TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX work_item_comment_ref ON work_item_comment (ref);

INSERT INTO work_item_link (from_ref, to_ref, link_type, created_at)
SELECT 'work_item:oxplow:tsk' || from_item_id, 'work_item:oxplow:tsk' || to_item_id,
       link_type, created_at
  FROM task_link;

INSERT INTO work_item_comment (id, ref, body, author, created_at)
SELECT 'task_note:' || id, 'work_item:oxplow:tsk' || task_id, body, author, created_at
  FROM task_note WHERE task_id IS NOT NULL;

-- oxplow's links and comments follow its tables, however they're written
-- (a link, an unlink, a cascade from the task or thread).
CREATE TRIGGER work_item_link_follows_task_link_insert AFTER INSERT ON task_link
BEGIN
    INSERT OR IGNORE INTO work_item_link (from_ref, to_ref, link_type, created_at)
    VALUES ('work_item:oxplow:tsk' || NEW.from_item_id, 'work_item:oxplow:tsk' || NEW.to_item_id,
            NEW.link_type, NEW.created_at);
END;
CREATE TRIGGER work_item_link_follows_task_link_delete AFTER DELETE ON task_link
BEGIN
    DELETE FROM work_item_link
     WHERE from_ref = 'work_item:oxplow:tsk' || OLD.from_item_id
       AND to_ref = 'work_item:oxplow:tsk' || OLD.to_item_id
       AND link_type = OLD.link_type;
END;
CREATE TRIGGER work_item_comment_follows_task_note_insert AFTER INSERT ON task_note
WHEN NEW.task_id IS NOT NULL
BEGIN
    INSERT INTO work_item_comment (id, ref, body, author, created_at)
    VALUES ('task_note:' || NEW.id, 'work_item:oxplow:tsk' || NEW.task_id, NEW.body, NEW.author,
            NEW.created_at);
END;
CREATE TRIGGER work_item_comment_follows_task_note_update AFTER UPDATE OF body ON task_note
WHEN NEW.task_id IS NOT NULL
BEGIN
    UPDATE work_item_comment SET body = NEW.body WHERE id = 'task_note:' || NEW.id;
END;
CREATE TRIGGER work_item_comment_follows_task_note_delete AFTER DELETE ON task_note
WHEN OLD.task_id IS NOT NULL
BEGIN
    DELETE FROM work_item_comment WHERE id = 'task_note:' || OLD.id;
END;
