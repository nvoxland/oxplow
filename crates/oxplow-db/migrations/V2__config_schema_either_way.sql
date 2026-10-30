-- The running version writes its configuration at its own schema version: a
-- later version moves it up, a rolled-back one down. Only the revision guards.
DROP TRIGGER extension_config_revision;
CREATE TRIGGER extension_config_revision
BEFORE UPDATE ON extension_config
WHEN NEW.extension IS NOT OLD.extension OR NEW.scope IS NOT OLD.scope
    OR NEW.revision != OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'extension configuration namespace or revision');
END;
