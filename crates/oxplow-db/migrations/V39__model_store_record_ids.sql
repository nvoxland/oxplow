-- A store's row says how a reader links its records without knowing the
-- model: the model's resource, the URI params that name the store and the
-- record, and whether its ids count in running text (a loose `tsk12`) or
-- only inside `[[…]]`. Restated with each publication, so the rows a
-- publication wrote before this are filled at the next one.
ALTER TABLE model_store_decl ADD COLUMN resource TEXT NOT NULL DEFAULT '';
ALTER TABLE model_store_decl ADD COLUMN store_param TEXT NOT NULL DEFAULT '';
ALTER TABLE model_store_decl ADD COLUMN record_key TEXT NOT NULL DEFAULT '';
ALTER TABLE model_store_decl ADD COLUMN loose INTEGER NOT NULL DEFAULT 0;
