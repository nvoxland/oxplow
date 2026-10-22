SELECT id, title, sort_index, created_at, updated_at,
       CAST(uri('oxplow/dashboard', 'id', 'dsh' || id) AS TEXT) AS uri
FROM source('dashboard')
