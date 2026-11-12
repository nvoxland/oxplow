-- Shared models (.context/models.md "Stores"): each live store of a model,
-- as the enabled extensions' fills and the running provider instances
-- state them. Restated with each publication, like `model_stage_decl`;
-- shared screens and the desktop read who fills a store and how its ids
-- look in text (a loose `tsk12` is a link).
CREATE TABLE model_store_decl (
  model      TEXT NOT NULL,
  store      TEXT NOT NULL,
  extension  TEXT NOT NULL,
  -- 1: a provider instance's (the read adapter writes it).
  provider   INTEGER NOT NULL DEFAULT 0,
  id_pattern TEXT,
  PRIMARY KEY (model, store)
) WITHOUT ROWID;
