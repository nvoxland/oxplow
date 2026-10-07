-- Per-table change generations, bumped by triggers inside the writing
-- transaction (`table_generations.rs`), and what each asset was last built
-- from, so an asset whose inputs haven't moved since needn't rebuild after
-- a restart (.context/semantic-layer.md "Assets").
CREATE TABLE table_generation (
    name TEXT PRIMARY KEY,
    gen  INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

ALTER TABLE asset_state ADD COLUMN built_from TEXT;
