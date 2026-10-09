-- Every enabled extension's contributions, as their descriptors say
-- (`oxplow_domain::contribution`): restated whole on each catalog change,
-- and read as `v_contribution`.
CREATE TABLE contribution (
    id TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    kind TEXT NOT NULL,
    local TEXT NOT NULL,
    origin TEXT NOT NULL,
    version TEXT,
    stability TEXT NOT NULL,
    contract_version INTEGER,
    runs_on_json TEXT NOT NULL,
    needs_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    global_name TEXT,
    aliases_json TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER
) STRICT;
