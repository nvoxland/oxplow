-- What site search indexes for each extension `v_extension` lists: its
-- name, and in the body its description and purpose. Extensions belong to
-- the project, not a stream.
SELECT CAST(e.uri AS TEXT) AS uri,
       CAST(e.name AS TEXT) AS title,
       CAST(trim(coalesce(e.description, '') || ' ' || coalesce(e.purpose, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('extension') e
