-- Private project-local override layers. JSON is opaque to the store:
-- manifest validation, live authority and secret references belong to the host.
CREATE TABLE extension_config (
    extension TEXT NOT NULL CHECK (length(CAST(extension AS BLOB)) BETWEEN 1 AND 128),
    scope TEXT NOT NULL CHECK (scope IN ('project', 'personal')),
    schema_version INTEGER NOT NULL CHECK (schema_version BETWEEN 1 AND 4294967295),
    revision INTEGER NOT NULL CHECK (revision > 0),
    values_json TEXT NOT NULL CHECK (COALESCE(
        json_valid(values_json)
        AND json_type(values_json) = 'object'
        AND length(CAST(values_json AS BLOB)) <= 1048576,
        0
    )),
    PRIMARY KEY (extension, scope)
) STRICT, WITHOUT ROWID;

-- Reset retains an empty object with an advanced stamp; it cannot recreate an
-- earlier revision and make an old compare-and-swap token valid again.
CREATE TRIGGER extension_config_revision
BEFORE UPDATE ON extension_config
WHEN NEW.extension IS NOT OLD.extension OR NEW.scope IS NOT OLD.scope
    OR NEW.revision != OLD.revision + 1
    OR NEW.schema_version < OLD.schema_version
BEGIN
    SELECT RAISE(ABORT, 'extension configuration namespace or revision');
END;
