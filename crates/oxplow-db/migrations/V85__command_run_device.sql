-- The device a person asked from (`dev-<uuid>`): the Mac app, or a paired
-- browser. NULL for a run no device asked for (an agent, an effect, a job,
-- a script), and for every run before this column.
ALTER TABLE command_run ADD COLUMN device_id TEXT;
