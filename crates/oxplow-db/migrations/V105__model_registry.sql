-- The model registry (P4.2, `.context/semantic-layer.md` "Models"). A
-- published view is a model: one SELECT in a file, compiled to a view at
-- every open. These tables are what the compiler records; `v_model*` read
-- them. Views are no longer created by migrations.

-- One row per compiled model, keyed by its view.
CREATE TABLE model (
    view        TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    owner       TEXT    NOT NULL,
    version     INTEGER NOT NULL,
    description TEXT    NOT NULL,
    -- The SELECT as compiled (refs and sources resolved).
    sql         TEXT    NOT NULL,
    compiled_at TEXT    NOT NULL
) STRICT;

-- What a model reads: another model (`ref`) or a table (`source`),
-- declared in its file and confirmed against what SQLite reports.
CREATE TABLE model_input (
    view  TEXT NOT NULL REFERENCES model(view) ON DELETE CASCADE,
    input TEXT NOT NULL,
    kind  TEXT NOT NULL CHECK (kind IN ('ref', 'source')),
    PRIMARY KEY (view, input)
) STRICT;

-- The columns a model promised at each version. Outlives the model row:
-- a changed contract at the same version is refused.
CREATE TABLE model_contract (
    view         TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    -- [{"name", "type", "doc"}] in column order.
    columns_json TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL,
    PRIMARY KEY (view, version)
) STRICT;

-- The last result of each declared test.
CREATE TABLE model_test (
    view   TEXT NOT NULL REFERENCES model(view) ON DELETE CASCADE,
    test   TEXT NOT NULL,
    state  TEXT NOT NULL CHECK (state IN ('passed', 'failed', 'error')),
    detail TEXT,
    ran_at TEXT NOT NULL,
    PRIMARY KEY (view, test)
) STRICT;
