-- What site search indexes for each page the launcher lists: its title,
-- and its description and keywords in the body. Pages belong to the
-- project, not a stream; hidden ones (opened from elsewhere) are left out.
SELECT CAST(p.uri AS TEXT) AS uri,
       CAST(p.title AS TEXT) AS title,
       CAST(trim(coalesce(p.description, '') || ' ' || coalesce(p.keywords, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('page') p
WHERE p.hidden = 0
