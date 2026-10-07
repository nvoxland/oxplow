-- The redo-rate signal counts efforts per work item, whichever work list
-- it's on (it was per oxplow task: `oxplow.task_effort` / `task.efforts`,
-- subject a bare task id). The measure keeps its id, so its facts and cube
-- rows stay attached; the facts name the item by ref, as `effort` does.

UPDATE measure
   SET key = 'oxplow.work_item_effort',
       title = 'Efforts per work item',
       subject_kind = 'work_item'
 WHERE key = 'oxplow.task_effort';

UPDATE fact
   SET subject_kind = 'work_item',
       subject_ref = 'work_item:oxplow:' || subject_ref
 WHERE measure_id = (SELECT id FROM measure WHERE key = 'oxplow.work_item_effort')
   AND subject_kind = 'task';

UPDATE metric_spec
   SET key = 'work_item.efforts',
       title = 'Efforts per work item',
       source_measure = 'oxplow.work_item_effort',
       description = 'Number of efforts spent on a work item (the redo-rate signal).'
 WHERE key = 'task.efforts';

UPDATE metric_catalog
   SET key = 'work_item.efforts', title = 'Efforts per work item'
 WHERE key = 'task.efforts';

UPDATE effort_metric_delta
   SET key = 'work_item.efforts', title = 'Efforts per work item'
 WHERE key = 'task.efforts';

UPDATE dashboard_item
   SET options_json = replace(options_json, '"task.efforts"', '"work_item.efforts"')
 WHERE options_json LIKE '%"task.efforts"%';
