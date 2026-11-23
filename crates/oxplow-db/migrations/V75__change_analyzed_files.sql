-- How many of a change's files its last deep analysis read, and how many it
-- changed: a change of more than the analysis's limit reads only some, and
-- the Review says so.
ALTER TABLE change ADD COLUMN analyzed_files INTEGER;
ALTER TABLE change ADD COLUMN changed_files INTEGER;
