-- Every contribution kind is stable, so a descriptor no longer carries a
-- per-kind stability. The table is restated whole on each catalog change.
ALTER TABLE contribution DROP COLUMN stability;
