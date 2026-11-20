-- What the symbol index was last filled with: the fingerprint of the
-- language set (the enabled extensions' languages). A different set (a pack
-- turned on or off, a language edited) restates the index.
CREATE TABLE symbol_index_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    languages TEXT NOT NULL
) STRICT;
