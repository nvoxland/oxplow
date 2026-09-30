-- History and branches read through the models (P5.B5,
-- `.context/vcs.md`): a branch says whether it is the repository's
-- default, tags are stored beside branches, and `v_commit` carries every
-- parent so a stream's history is a recursive read from its head.
ALTER TABLE git_branch ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;

CREATE TABLE git_tag (
    name TEXT PRIMARY KEY,
    sha TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
