-- An effort's impact on a work item is kind `work_item` (it was `task`):
-- the work list's item, whichever list, named as its ref or one of that
-- list's own ids. The stored ones were oxplow's tasks, named `tsk42`,
-- `42` or `oxplow:tsk42`: they become their canonical refs.
UPDATE effort
   SET impacts_json = (
       SELECT json_group_array(
                  CASE WHEN json_extract(i.value, '$.kind') = 'task'
                       THEN json_set(
                                i.value,
                                '$.kind', 'work_item',
                                '$.id',
                                CASE
                                    WHEN json_extract(i.value, '$.id') GLOB 'oxplow:tsk[0-9]*'
                                        THEN 'work_item:' || json_extract(i.value, '$.id')
                                    WHEN json_extract(i.value, '$.id') GLOB 'tsk[0-9]*'
                                        THEN 'work_item:oxplow:' || json_extract(i.value, '$.id')
                                    WHEN json_extract(i.value, '$.id') GLOB '[0-9]*'
                                        THEN 'work_item:oxplow:tsk' || json_extract(i.value, '$.id')
                                    ELSE json_extract(i.value, '$.id')
                                END)
                       ELSE json(i.value) END)
         FROM json_each(effort.impacts_json) i)
 WHERE impacts_json IS NOT NULL
   AND impacts_json <> ''
   AND json_valid(impacts_json)
   AND EXISTS (SELECT 1 FROM json_each(effort.impacts_json) i
                WHERE json_extract(i.value, '$.kind') = 'task');
