-- Extension entity views are models (P4.9, `.context/semantic-layer.md`
-- "Models"): the registry says who owns every published view, and how it
-- is made — `sql` (compiled from a model file at every open) or `entity`
-- (created by an extension's source when it syncs, kept across opens).
ALTER TABLE model ADD COLUMN kind TEXT NOT NULL DEFAULT 'sql'
    CHECK (kind IN ('sql', 'entity'));

-- The entity views synced before the registry knew them: a view named for
-- an extension with source state that reads that extension's own table.
INSERT INTO model (view, name, owner, version, description, sql, compiled_at, kind)
SELECT v.name,
       substr(v.name, length('v_' || replace(e.extension, '-', '_') || '_') + 1),
       e.extension,
       1,
       'Entity synced by the `' || e.extension || '` extension''s sources.',
       substr(v.sql, instr(upper(v.sql), ' AS SELECT ') + 4),
       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
       'entity'
FROM (SELECT DISTINCT extension FROM ext_source_state) e
JOIN sqlite_master v
  ON v.type = 'view'
 AND substr(v.name, 1, length('v_' || replace(e.extension, '-', '_') || '_'))
     = 'v_' || replace(e.extension, '-', '_') || '_'
 AND instr(v.sql, 'ext__' || replace(e.extension, '-', '_') || '__') > 0
WHERE v.name NOT IN (SELECT view FROM model);

INSERT INTO model_input (view, input, kind)
SELECT m.view, t.name, 'source'
FROM model m
JOIN sqlite_master t
  ON t.type = 'table'
 AND t.name = 'ext__' || replace(m.owner, '-', '_') || '__' || m.name
WHERE m.kind = 'entity';
