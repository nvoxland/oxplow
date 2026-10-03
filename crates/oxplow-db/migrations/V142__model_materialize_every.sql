-- P8.B2: a model's freshness policy may be a clock — `every 1h` — as well
-- as `on_change`. SQLite can't change a CHECK in place, so `model` is
-- rebuilt; its dependents (`model_input`, `model_test`) cascade on the
-- drop, so they're kept aside and put back.
CREATE TEMP TABLE kept_model_input AS SELECT * FROM model_input;
CREATE TEMP TABLE kept_model_test AS SELECT * FROM model_test;

CREATE TABLE model_new (
    view        TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    owner       TEXT    NOT NULL,
    version     INTEGER NOT NULL,
    description TEXT    NOT NULL,
    sql         TEXT    NOT NULL,
    compiled_at TEXT    NOT NULL,
    kind        TEXT    NOT NULL DEFAULT 'sql' CHECK (kind IN ('sql', 'entity')),
    materialize TEXT    CHECK (materialize = 'on_change' OR materialize LIKE 'every %')
) STRICT;
INSERT INTO model_new
SELECT view, name, owner, version, description, sql, compiled_at, kind, materialize FROM model;
DROP TABLE model;
ALTER TABLE model_new RENAME TO model;

INSERT INTO model_input SELECT * FROM kept_model_input;
INSERT INTO model_test SELECT * FROM kept_model_test;
DROP TABLE kept_model_input;
DROP TABLE kept_model_test;
