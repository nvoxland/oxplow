-- Git history and branches in the semantic layer (tsk318). The commit
-- indexer stores what it already reads for page_ref edges; the branch
-- refresh restates git_branch on boot and on ref changes.
CREATE TABLE git_commit (
    sha TEXT PRIMARY KEY,
    author TEXT NOT NULL,
    email TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    parents_json TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX idx_git_commit_time ON git_commit(committed_at);

CREATE TABLE git_commit_file (
    sha TEXT NOT NULL REFERENCES git_commit(sha) ON DELETE CASCADE,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (sha, path)
);
CREATE INDEX idx_git_commit_file_path ON git_commit_file(path);

CREATE TABLE git_branch (
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('local', 'remote')),
    remote TEXT,
    head_sha TEXT,
    stream_id INTEGER,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (kind, remote, name)
);

CREATE VIEW v_commit AS
SELECT sha, author, email, committed_at, subject, body,
       json_extract(parents_json, '$[0]') AS first_parent,
       json_array_length(parents_json) AS parent_count
FROM git_commit;

CREATE VIEW v_commit_file AS
SELECT sha, path, status, additions, deletions
FROM git_commit_file;

-- Tasks a commit's message mentions (the indexer's page_ref edges).
CREATE VIEW v_commit_task AS
SELECT source_id AS sha, CAST(substr(target_id, 4) AS INTEGER) AS task_id
FROM page_ref
WHERE source_kind = 'git-commit' AND target_kind = 'task' AND target_id LIKE 'tsk%';

CREATE VIEW v_branch AS
SELECT name, kind, remote, head_sha, stream_id, updated_at
FROM git_branch;
