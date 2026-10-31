-- What site search indexes for each setting `v_config_key` lists: its key,
-- and in the body what it is for. Settings belong to the project, not a
-- stream.
SELECT CAST(uri('oxplow/config', 'key', k.key) AS TEXT) AS uri,
       CAST(k.key AS TEXT) AS title,
       CAST(coalesce(k.doc, '') AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('config_key') k
