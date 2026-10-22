-- What site search indexes for each lens: its title, and its description in
-- the body. Lenses belong to the project, not a stream. Hidden lenses
-- (the agent's typed tools) are included.
SELECT CAST(l.uri AS TEXT) AS uri,
       CAST(l.title AS TEXT) AS title,
       CAST(coalesce(l.description, '') AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('lens') l
