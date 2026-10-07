-- Each capability's implementations say how a person names them, how they're
-- loaded, whether a chosen one is there at all, and on the active row why it's
-- the one (a person's choice, the project's, the default, or a fallback). The
-- app's capability registry restates the table whole.
ALTER TABLE capability_provider ADD COLUMN title TEXT NOT NULL DEFAULT '';
ALTER TABLE capability_provider ADD COLUMN source TEXT NOT NULL DEFAULT 'core';
ALTER TABLE capability_provider ADD COLUMN available INTEGER NOT NULL DEFAULT 1;
ALTER TABLE capability_provider ADD COLUMN chosen_by TEXT;
