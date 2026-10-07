-- What the desktop's work-item screens read, whichever list is active.
--
-- A comment on a work item targets it as the item page names it: kind
-- `work_item`, id `<provider>:<id>`. The task page targeted oxplow's tasks
-- as kind `task`, id `tsk<n>`.
UPDATE comment
   SET target_kind = 'work_item',
       target_id = 'oxplow:' || target_id
 WHERE target_kind = 'task'
   AND target_id GLOB 'tsk[0-9]*';

-- A work list's id pattern beside it, so a screen recognizes its ids in
-- text (`[[tsk42]]`) as core's vocabulary does. Restated with the rows.
ALTER TABLE capability_provider ADD COLUMN id_pattern TEXT;
