-- A work item's page refs come from the interface, for every list
-- (`.context/work-items.md`): its links take the list's own link types
-- (the CHECK named oxplow's six), and its comments' mentions are the
-- item's own edges (`comment_*` ref types), no longer a `task_note`
-- source's. The boot repair restates them from `work_item_comment`.

DROP TRIGGER work_item_link_follows_task_link_insert;
DROP TRIGGER work_item_link_follows_task_link_delete;

CREATE TABLE work_item_link_new (
    from_ref TEXT NOT NULL,
    to_ref TEXT NOT NULL,
    link_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_ref, to_ref, link_type)
);
INSERT INTO work_item_link_new (from_ref, to_ref, link_type, created_at)
SELECT from_ref, to_ref, link_type, created_at FROM work_item_link;
DROP TABLE work_item_link;
ALTER TABLE work_item_link_new RENAME TO work_item_link;

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

DELETE FROM page_ref WHERE source_kind = 'task_note';
