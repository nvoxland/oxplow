-- Shared models (.context/models.md): the per-item fields of any model,
-- kept apart by the namespace that wrote them, and the fields each
-- extension declares. A model's own rows are in its `model__<ns>__<name>`
-- table, made from its declaration.
CREATE TABLE model_field_value (
  model     TEXT NOT NULL,
  uri       TEXT NOT NULL,
  namespace TEXT NOT NULL,
  name      TEXT NOT NULL,
  value     TEXT NOT NULL,
  PRIMARY KEY (model, uri, namespace, name)
) WITHOUT ROWID;

CREATE INDEX model_field_value_uri ON model_field_value (uri);

CREATE TABLE model_field_decl (
  model       TEXT NOT NULL,
  namespace   TEXT NOT NULL,
  name        TEXT NOT NULL,
  extension   TEXT NOT NULL,
  title       TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('enum', 'text', 'number')),
  values_json TEXT,
  read_only   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (model, namespace, name)
) WITHOUT ROWID;
