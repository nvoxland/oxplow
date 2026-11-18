-- Zones are gone: oxplow no longer classifies a change's files or imports by
-- a project's zone table. A change keeps its files and its imports.
ALTER TABLE change_file DROP COLUMN zone;
ALTER TABLE change_import DROP COLUMN from_zone;
ALTER TABLE change_import DROP COLUMN to_zone;
ALTER TABLE change_import DROP COLUMN cross_zone;
