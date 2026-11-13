-- Shared models (.context/models.md "Stages"): each store's workflow, as
-- the enabled extensions declare it (`fills.stages`) and the running
-- provider instances state it. Restated with each publication, like
-- `model_field_decl`; shared screens draw a store's columns from it.
CREATE TABLE model_stage_decl (
  model     TEXT NOT NULL,
  store     TEXT NOT NULL,
  -- Its place in the store's workflow, from 0.
  position  INTEGER NOT NULL,
  id        TEXT NOT NULL,
  title     TEXT NOT NULL,
  bucket    TEXT NOT NULL
            CHECK (bucket IN ('backlog', 'todo', 'in_progress', 'blocked', 'done', 'canceled')),
  extension TEXT NOT NULL,
  PRIMARY KEY (model, store, id)
) WITHOUT ROWID;
