-- oxplow's tasks reach the work-item interface like any list: through the
-- `work_item.recorded` their verbs answer with, upserted by the
-- `work_items.project` consumer (`.context/work-items.md`). Nothing writes
-- `work_item*` from the task tables any more: their triggers go, and the
-- rows already there take the record's shape — comment ids `<ref>#not<n>`,
-- `native` the list's declared fields.

DROP TRIGGER work_item_follows_task_delete;
DROP TRIGGER work_item_link_follows_task_link_insert;
DROP TRIGGER work_item_link_follows_task_link_delete;
DROP TRIGGER work_item_comment_follows_task_note_insert;
DROP TRIGGER work_item_comment_follows_task_note_update;
DROP TRIGGER work_item_comment_follows_task_note_delete;

UPDATE work_item_comment
   SET id = ref || '#not' || substr(id, length('task_note:') + 1)
 WHERE id LIKE 'task_note:%';

UPDATE work_item
   SET native = json_object('priority', json_extract(native, '$.priority'),
                            'author', json_extract(native, '$.author'))
 WHERE provider = 'oxplow';
