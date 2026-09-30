-- A run's result is part of its record (P5.B6): an External command's
-- result is what the system it drove reported — a VCS operation's
-- outcome, conflicts included — which no event carries.
ALTER TABLE command_audit ADD COLUMN result_json TEXT;
