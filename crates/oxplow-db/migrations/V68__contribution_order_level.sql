-- Where an effect or a collector runs among what one event reaches, as the
-- catalog compiled its `after:` (0 first); NULL for a kind that isn't
-- ordered. `after_json` now holds the resolved refs.
ALTER TABLE contribution ADD COLUMN order_level INTEGER;
