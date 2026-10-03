-- tsk865: `measure.component_role` has been dead since tsk15 (ratio
-- components ride each fact's num/den) and no config sets it any more.
ALTER TABLE measure DROP COLUMN component_role;
