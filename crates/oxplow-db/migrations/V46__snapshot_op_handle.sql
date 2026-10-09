-- A snapshot provider's own name for the state a take marked. Core gives
-- it back as the parent of the provider's next mark and as the points it
-- asks what changed between. Takes by core's capture pipeline name none.
ALTER TABLE snapshot_op ADD COLUMN handle TEXT;
