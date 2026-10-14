-- A change's function rows are keyed by the function's symbol ref, so
-- overloads and same-named functions in one file stay apart. The rows are
-- derived: they are recomputed when a change is next analyzed.

DROP TABLE change_function;
CREATE TABLE change_function (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    -- `symbol:<path>/<id>`
    ref TEXT NOT NULL,
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
    PRIMARY KEY (change_id, path, ref)
);
