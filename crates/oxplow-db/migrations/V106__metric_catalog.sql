-- The resolved metric catalog as data (P4.7): what the metrics service
-- assembles from the bundled gauges, producers, entity metrics, specs and
-- this project's config, rewritten on every reseed so the catalog reads
-- through SQL (`v_metric_catalog`) like everything else.
CREATE TABLE metric_catalog (
    key        TEXT    PRIMARY KEY,
    title      TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    language   TEXT,
    scope      TEXT    NOT NULL,
    enabled    INTEGER NOT NULL,
    target     REAL,
    trigger    TEXT    NOT NULL,
    toggleable INTEGER NOT NULL,
    category   TEXT
) STRICT;
