-- Record the git branch a snapshot was captured on, so callers can
-- distinguish snapshots taken before vs. after a branch switch within
-- the same stream's worktree (the stream_id alone can't tell them
-- apart). Populated at capture time from the live HEAD; NULL for
-- pre-V42 rows and for captures where the directory isn't a git repo
-- or HEAD is detached.
ALTER TABLE snapshot ADD COLUMN git_branch TEXT;
