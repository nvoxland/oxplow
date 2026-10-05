-- tsk1041: the thread that filed an outside tracker's item (an oxplow
-- task's thread is the task's own). Set at the item's first record, from
-- the record's thread anchor; a later restatement doesn't move it.
ALTER TABLE work_item ADD COLUMN filed_in_thread INTEGER;
