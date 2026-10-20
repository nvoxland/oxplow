-- A lens row names its source file, the one to edit (NULL when no source
-- is here), not the file it runs from: a deployed extension runs a copy
-- in the store that the next deploy replaces. The table is restated whole
-- with each extension generation, so it is rebuilt empty.
DROP TABLE lens;
CREATE TABLE lens (
    id TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    viz TEXT NOT NULL,
    params_json TEXT NOT NULL,
    hidden INTEGER NOT NULL,
    launcher_category TEXT,
    needs_json TEXT NOT NULL,
    source TEXT
) STRICT;
