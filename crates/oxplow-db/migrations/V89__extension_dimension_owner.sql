-- Dimensions an extension declares (tsk328). Stored as scope 'global' (the
-- V43 CHECK allows only built-in / global / project) with the owning
-- extension here, like measures and specs in V84; pruning keys on it.
ALTER TABLE dimension ADD COLUMN extension TEXT;
