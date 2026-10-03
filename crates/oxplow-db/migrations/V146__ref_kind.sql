-- P8.D6: every kind of ref the running vocabulary knows — core's and
-- those extensions declare (`ref_kinds:`) — restated whole on each
-- rebuild, read as `v_ref_kind`. An extension's kind names how to show
-- one: its label and icon, the model that titles it (`resolve`) and the
-- page that opens it. Unlike event types, a removed extension's kinds
-- leave: a ref to one is unrecognized again.
CREATE TABLE ref_kind (
    kind TEXT PRIMARY KEY,
    -- NULL for a core kind.
    extension TEXT,
    label TEXT,
    id_pattern TEXT NOT NULL,
    revisioned INTEGER NOT NULL CHECK (revisioned IN (0, 1)),
    -- JSON array of its `[[prefix:…]]` sugar.
    wikilinks TEXT NOT NULL,
    resolve TEXT,
    page TEXT,
    icon TEXT
) STRICT;
