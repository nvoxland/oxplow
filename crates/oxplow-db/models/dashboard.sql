SELECT id, title, sort_index, created_at, updated_at,
       CAST('dashboard:dsh' || id AS TEXT) AS ref
FROM source('dashboard')
