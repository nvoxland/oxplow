SELECT m.view, m.name, m.owner, m.kind, m.version, m.description, m.sql, m.compiled_at,
       m.materialize, a.computed_at, a.events_to
FROM source('model') m
LEFT JOIN source('asset_state') a ON a.asset = m.view
