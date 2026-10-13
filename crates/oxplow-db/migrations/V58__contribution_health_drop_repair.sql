-- The repair item is no longer core's bookkeeping: the bundled repair
-- effect files it, and the health view finds the open one by its title.
ALTER TABLE contribution_health DROP COLUMN repair_item;
ALTER TABLE contribution_health DROP COLUMN repair_seq;
