-- An implementation's own fields (`v_capability_provider.fields`): what a
-- work list keeps in `native` beyond the interface's columns (oxplow's
-- tasks: priority), so screens render and edit them without knowing which
-- list is active.
ALTER TABLE capability_provider ADD COLUMN fields_json TEXT NOT NULL DEFAULT '[]';
