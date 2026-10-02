-- P8.B1: a model version's contract includes its key — the columns whose
-- values name one row. Every version recorded before keys promised none.
ALTER TABLE model_contract ADD COLUMN key_json TEXT NOT NULL DEFAULT '[]';
