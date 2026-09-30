-- tsk557: a changed file the language server failed on (an error, or no
-- answer in time) counts against the bound and is recorded, instead of
-- being invisible.
ALTER TABLE symbol_capture ADD COLUMN files_failed INTEGER NOT NULL DEFAULT 0;
