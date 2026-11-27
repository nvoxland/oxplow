-- What a person calls a contribution in the app (a command's `ui.label`)
-- and other words they may look for it by (its `ui.group` and
-- `ui.keywords`): what site search names a command by. Restated with the
-- next extension generation.
ALTER TABLE contribution ADD COLUMN label TEXT;
ALTER TABLE contribution ADD COLUMN keywords TEXT;
