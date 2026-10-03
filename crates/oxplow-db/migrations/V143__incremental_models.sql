-- P8.B4: a model may be kept incrementally — `incremental <column>` —
-- and an asset's last recompute says how it ran: `full` (refilled whole)
-- or `incremental` (rows past the watermark appended), the watermark it
-- reached and how many rows the asset holds. `model` is rebuilt for the
-- wider CHECK; its dependents cascade on the drop, so they're put back.
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
    materialize TEXT    CHECK (materialize = 'on_change'
                               OR materialize LIKE 'every %'
                               OR materialize LIKE 'incremental %')
) STRICT;
INSERT INTO model_new
SELECT view, name, owner, version, description, sql, compiled_at, kind, materialize FROM model;
DROP TABLE model;
ALTER TABLE model_new RENAME TO model;

INSERT INTO model_input SELECT * FROM kept_model_input;
INSERT INTO model_test SELECT * FROM kept_model_test;
DROP TABLE kept_model_input;
DROP TABLE kept_model_test;

ALTER TABLE asset_state ADD COLUMN mode TEXT CHECK (mode IN ('full', 'incremental'));
ALTER TABLE asset_state ADD COLUMN watermark INTEGER;
ALTER TABLE asset_state ADD COLUMN row_count INTEGER;
