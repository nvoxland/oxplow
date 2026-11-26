-- A branch's head commit, as a list shows it: its subject and when it was
-- made (RFC 3339), so a branch no stream uses still says what it holds.
ALTER TABLE git_branch ADD COLUMN subject TEXT;
ALTER TABLE git_branch ADD COLUMN committed_at TEXT;
