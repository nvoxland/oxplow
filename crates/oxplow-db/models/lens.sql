SELECT CAST(uri('oxplow/lens', 'lens', id) AS TEXT) AS ref, extension, slug, title, description, viz,
       params_json AS params, hidden, launcher_category, needs_json AS needs, source
FROM source('lens')
