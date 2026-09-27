-- Change analysis, stored (tsk307). A `change` is one diff: a commit
-- (parent → sha), an effort (start → end snapshot, or → working tree while
-- open), or the working tree (HEAD → disk). Core's producer analyzes it once
-- (or again when the working tree moves) and lenses read the results.

CREATE TABLE change (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working')),
    -- The sha, the effort id, or '' for the working tree.
    target TEXT NOT NULL,
    base_label TEXT,
    head_label TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    error TEXT,
    computed_at TEXT,
    UNIQUE (stream_id, kind, target)
);

CREATE TABLE change_file (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    additions INTEGER NOT NULL,
    deletions INTEGER NOT NULL,
    zone TEXT,
    is_test INTEGER NOT NULL,
    interest REAL NOT NULL,
    interest_reasons TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (change_id, path)
);

CREATE TABLE change_function (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    container TEXT NOT NULL,
    name TEXT NOT NULL,
    -- `added` | `deleted` | `modified`
    status TEXT NOT NULL,
    signature_changed INTEGER NOT NULL,
    body_changed INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    visibility TEXT NOT NULL,
    is_test INTEGER NOT NULL,
    complexity REAL,
    length INTEGER,
    params_before INTEGER,
    params_after INTEGER,
    complexity_delta REAL,
    length_delta INTEGER,
    added_lines INTEGER,
    deleted_lines INTEGER,
    modified_lines INTEGER,
    churn_share REAL,
    PRIMARY KEY (change_id, path, container, name)
);

CREATE TABLE change_import (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    module TEXT NOT NULL,
    -- `added` | `removed`
    direction TEXT NOT NULL,
    start_line INTEGER,
    from_zone TEXT,
    to_zone TEXT,
    cross_zone INTEGER NOT NULL
);

CREATE TABLE change_co_change (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    -- `usual-co-changers-absent` | `dormant`
    reason TEXT NOT NULL,
    expected TEXT,
    dormant_days INTEGER,
    PRIMARY KEY (change_id, path)
);

CREATE TABLE change_duplicate (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    lines INTEGER NOT NULL,
    peer_path TEXT NOT NULL,
    peer_start_line INTEGER NOT NULL,
    peer_end_line INTEGER NOT NULL
);

CREATE VIEW v_change AS
SELECT id, stream_id, kind, target, base_label, head_label, status, error, computed_at
FROM change;

CREATE VIEW v_change_file AS
SELECT change_id, path, status, additions, deletions, zone, is_test, interest, interest_reasons
FROM change_file;

CREATE VIEW v_change_function AS
SELECT change_id, path, container, name, status, signature_changed, body_changed, start_line,
       visibility, is_test, complexity, length, params_before, params_after, complexity_delta,
       length_delta, added_lines, deleted_lines, modified_lines, churn_share
FROM change_function;

CREATE VIEW v_change_import AS
SELECT change_id, path, module, direction, start_line, from_zone, to_zone, cross_zone
FROM change_import;

CREATE VIEW v_change_co_change AS
SELECT change_id, path, reason, expected, dormant_days FROM change_co_change;

CREATE VIEW v_change_duplicate AS
SELECT change_id, path, start_line, end_line, lines, peer_path, peer_start_line, peer_end_line
FROM change_duplicate;
