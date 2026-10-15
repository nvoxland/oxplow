-- The lenses and extensions the catalog holds, restated whole with each
-- extension generation and read as `v_lens` and `v_extension`.
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
    path TEXT NOT NULL
) STRICT;

CREATE TABLE extension_listing (
    name TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    description TEXT NOT NULL,
    path TEXT NOT NULL,
    origin TEXT NOT NULL,
    version TEXT,
    enabled INTEGER NOT NULL,
    sharing TEXT NOT NULL,
    purpose TEXT,
    source_git TEXT,
    source_sha TEXT,
    errors_json TEXT NOT NULL,
    warnings_json TEXT NOT NULL
) STRICT;
