-- P8.D3: every event type the vocabulary has registered, at each version,
-- with the schema it was first registered with. A declared (extension)
-- type's schema at a recorded `type@v` is a contract: a changed one is
-- refused (a new shape is a new `v`). `registered` is whether the running
-- vocabulary has it now; an extension's removed types stay listed, and
-- their logged rows stay readable. Read as `v_event_type`.
CREATE TABLE event_type_contract (
    event_type TEXT NOT NULL,
    v INTEGER NOT NULL CHECK (v >= 1),
    -- NULL for a core type.
    extension TEXT,
    schema_json TEXT NOT NULL,
    summary TEXT,
    registered INTEGER NOT NULL CHECK (registered IN (0, 1)),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (event_type, v)
) STRICT;
