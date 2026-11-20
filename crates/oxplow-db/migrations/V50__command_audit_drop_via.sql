-- `command_audit.via` named what an actor ran a command through (the
-- Assistant, acting for a person). The Assistant is gone and nothing else
-- acts through anything, so the column goes. The rows stay.
ALTER TABLE command_audit DROP COLUMN via;
