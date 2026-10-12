-- The core sub-components an extension may replace, with each one's
-- versioned props contract: oxplow's own table, restated whole at every
-- vocabulary rebuild (as `ref_kind` is), read as `v_replaceable`.
CREATE TABLE replaceable (
    target TEXT PRIMARY KEY,
    capability TEXT NOT NULL,
    bound INTEGER NOT NULL CHECK (bound IN (0, 1)),
    version INTEGER NOT NULL CHECK (version >= 1),
    -- JSON array of prop names.
    props TEXT NOT NULL,
    label TEXT NOT NULL
) STRICT;
