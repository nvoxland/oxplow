-- Every resource a URI may name (`oxplow:<extension>/<resource>`), as
-- the extensions declare them (`resources:`, oxplow-foundation's
-- included): restated whole at every vocabulary rebuild, read as
-- `v_resource`.
CREATE TABLE resource (
    resource TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    label TEXT NOT NULL,
    -- JSON object: param name -> { required, pattern }.
    params TEXT NOT NULL,
    additional INTEGER NOT NULL CHECK (additional IN (0, 1)),
    resolve TEXT,
    title TEXT,
    searchable TEXT,
    icon TEXT,
    -- JSON { prefix, param }, or NULL.
    wikilink TEXT
) STRICT;
