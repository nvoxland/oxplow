-- The page catalog beside the lens listing: every page file of every
-- enabled extension, its answers and its params. Restated whole with each
-- extension generation, as `lens` is.
CREATE TABLE page (
    uri TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    label TEXT,
    icon TEXT,
    category TEXT,
    keywords TEXT,
    params_json TEXT NOT NULL,
    renders_json TEXT NOT NULL,
    is_default INTEGER NOT NULL,
    pinned INTEGER NOT NULL,
    hidden INTEGER NOT NULL,
    tool TEXT,
    needs_json TEXT NOT NULL,
    source TEXT,
    deployment TEXT NOT NULL
) STRICT;

CREATE TABLE page_answer (
    page_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    question TEXT NOT NULL,
    about TEXT,
    params_json TEXT NOT NULL,
    PRIMARY KEY (page_uri, position)
) STRICT;

CREATE TABLE page_param (
    page_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    role TEXT NOT NULL,
    label TEXT,
    default_json TEXT,
    options_json TEXT NOT NULL,
    PRIMARY KEY (page_uri, position)
) STRICT;
