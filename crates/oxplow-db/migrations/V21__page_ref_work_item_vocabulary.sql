-- The page-ref vocabulary speaks of work items, whichever list they're
-- on: a mention is `work_item_mention` / `summary_work_item_mention`, a
-- link `work_item_link:<type>` (they were `task_…`). A row already in
-- its new spelling (a restate since) wins over the old one.
DELETE FROM page_ref
 WHERE ref_type IN ('task_body_mention', 'summary_task_mention')
   AND EXISTS (SELECT 1 FROM page_ref n
                WHERE n.source_kind = page_ref.source_kind AND n.source_id = page_ref.source_id
                  AND n.target_kind = page_ref.target_kind AND n.target_id = page_ref.target_id
                  AND n.ref_type = CASE page_ref.ref_type
                                       WHEN 'task_body_mention' THEN 'work_item_mention'
                                       ELSE 'summary_work_item_mention' END);
UPDATE page_ref SET ref_type = 'work_item_mention' WHERE ref_type = 'task_body_mention';
UPDATE page_ref SET ref_type = 'summary_work_item_mention' WHERE ref_type = 'summary_task_mention';
DELETE FROM page_ref
 WHERE ref_type LIKE 'task_link:%'
   AND EXISTS (SELECT 1 FROM page_ref n
                WHERE n.source_kind = page_ref.source_kind AND n.source_id = page_ref.source_id
                  AND n.target_kind = page_ref.target_kind AND n.target_id = page_ref.target_id
                  AND n.ref_type = 'work_item_link:' || substr(page_ref.ref_type, 11));
UPDATE page_ref SET ref_type = 'work_item_link:' || substr(ref_type, 11)
 WHERE ref_type LIKE 'task_link:%';
