-- A snapshot names the VCS revision its tree equals the way everything
-- else does (P5.B3, `oxplow_domain::vcs::Revision`): `git:<sha>` in
-- `revision`, replacing the bare sha in `git_commit`. Its branch is the
-- VCS's, so the column is `branch`.
ALTER TABLE snapshot RENAME COLUMN git_commit TO revision;
UPDATE snapshot SET revision = 'git:' || revision WHERE revision IS NOT NULL;
ALTER TABLE snapshot RENAME COLUMN git_branch TO branch;
