-- P7.B1: each asset's last recompute. An asset is derived data computed
-- from tables (the metric cube; a model materialized on change): when a
-- commit touches one of its inputs it is recomputed, and this row says
-- when, how far into the event log its inputs were (`events_to`: the
-- log's highest seq as it began), against which snapshot (when it has
-- one) and how long it took.
CREATE TABLE asset_state (
    asset TEXT PRIMARY KEY,
    computed_at TEXT NOT NULL,
    events_to INTEGER NOT NULL,
    snapshot_id INTEGER,
    elapsed_ms INTEGER NOT NULL
) STRICT;
