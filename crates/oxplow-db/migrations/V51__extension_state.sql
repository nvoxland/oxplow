-- Host-owned workflow state and rebuildable caches. Project isolation is
-- the database; the host derives extension ownership from its RunContext.
-- Retain rows across disable/uninstall and reset through revisioned CAS,
-- so a stale request cannot recreate deleted values after a reset.
CREATE TABLE extension_state (
    extension TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('project', 'personal')),
    area TEXT NOT NULL CHECK (area IN ('state', 'cache')),
    schema_version INTEGER NOT NULL
        CHECK (schema_version > 0 AND schema_version <= 4294967295),
    revision INTEGER NOT NULL CHECK (revision > 0),
    values_json TEXT NOT NULL
        CHECK (json_valid(values_json)
            AND json_type(values_json) = 'object'
            AND length(CAST(values_json AS BLOB)) <= 1048576),
    PRIMARY KEY (extension, scope, area)
) STRICT, WITHOUT ROWID;
