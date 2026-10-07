-- Each row carries its capability as core declares it — how a person names
-- it, whether a project chooses it, whether it may be none — so Settings
-- lists the choices from the model alone.
ALTER TABLE capability_provider ADD COLUMN capability_title TEXT NOT NULL DEFAULT '';
ALTER TABLE capability_provider ADD COLUMN choosable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE capability_provider ADD COLUMN optional INTEGER NOT NULL DEFAULT 0;
