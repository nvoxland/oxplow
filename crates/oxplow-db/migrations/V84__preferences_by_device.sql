-- What a device remembers is its own (`.context/data-model.md` "ui_preference").
-- A custom component's memory is extension data and stays shared: it moves,
-- copied, into `component_memory`. The other rows go: every device starts from
-- the defaults.
CREATE TABLE component_memory (
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (scope, key)
) STRICT;
INSERT INTO component_memory (scope, key, value_json, updated_at)
    SELECT scope, key, value_json, updated_at FROM ui_preference
    WHERE scope LIKE 'component:%';
CREATE TABLE ui_preference_new (
    device_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (device_id, scope, key)
) STRICT;
DROP TABLE ui_preference;
ALTER TABLE ui_preference_new RENAME TO ui_preference;
