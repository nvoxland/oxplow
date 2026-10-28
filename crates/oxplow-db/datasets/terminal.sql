SELECT id, stream_id, title, position, created_at,
       CAST(uri('oxplow/terminal', 'id', 'trm' || id) AS TEXT) AS uri
FROM source('terminal')
