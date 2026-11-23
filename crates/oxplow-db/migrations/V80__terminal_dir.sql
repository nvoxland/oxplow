-- The directory a terminal's shell starts in (Open In ▸ Terminal): a path
-- relative to the stream's worktree, or an absolute one. NULL is the
-- worktree itself.
ALTER TABLE terminal ADD COLUMN dir TEXT;
