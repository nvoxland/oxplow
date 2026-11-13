-- A body's mention of a record of a shared model (a task, a page) is one
-- ref type: core no longer tells a model's kind from its ref type.
UPDATE page_ref SET ref_type = 'record_mention'
 WHERE ref_type IN ('wikilink', 'task_mention');
UPDATE page_ref SET ref_type = 'comment_record_mention'
 WHERE ref_type IN ('comment_wikilink', 'comment_task_mention');
