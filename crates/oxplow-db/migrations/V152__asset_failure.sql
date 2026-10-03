-- tsk781: an asset's last recompute failure — a materialized model whose
-- SELECT emits one key twice, a query error — until a recompute succeeds.
-- Read through `v_asset`, so a model that stopped updating says so rather
-- than only logging it.
CREATE TABLE asset_failure (
    asset TEXT PRIMARY KEY,
    failed_at TEXT NOT NULL,
    error TEXT NOT NULL
) STRICT;
