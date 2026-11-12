-- Shared models (.context/models.md "Quarantine"): a row a write skipped
-- because it failed its checks stays an open problem until its key is
-- restated good, deleted, renamed away, or left out of a snapshot of its
-- store. An incremental store states a row once, so the run's report alone
-- would lose it.
CREATE TABLE model_quarantine (
  model   TEXT NOT NULL,
  store   TEXT NOT NULL,
  -- The row's key, or `#<position>` for a row with no usable key.
  key     TEXT NOT NULL,
  uri     TEXT,
  message TEXT NOT NULL,
  -- The writer's event source (the collector's URI).
  source  TEXT NOT NULL,
  at      TEXT NOT NULL,
  PRIMARY KEY (model, store, key)
) WITHOUT ROWID;
