-- Which of a working tree's changed files are staged (in the index) and
-- which are untracked; NULL for the other kinds of change.
ALTER TABLE change_file ADD COLUMN staged INTEGER;
ALTER TABLE change_file ADD COLUMN untracked INTEGER;
