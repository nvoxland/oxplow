-- The host a stream's worktree is on (`HostId`; `.context/vcs.md` "Around
-- the provider"). Every stream so far is on the machine oxplow runs on.
ALTER TABLE streams ADD COLUMN host TEXT NOT NULL DEFAULT 'local';
