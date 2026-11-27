-- What site search indexes for each page the launcher lists: its title,
-- and its description and keywords in the body; and each section it shows
-- by a switch beside the one it opens on, titled "<page>: <section>",
-- opening the page on it. Pages belong to the project, not a stream; a
-- page with no category (opened from elsewhere), a part and a query are
-- left out.
SELECT CAST(p.uri AS TEXT) AS uri,
       CAST(p.title AS TEXT) AS title,
       CAST(trim(coalesce(p.description, '') || ' ' || coalesce(p.keywords, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('page') p
WHERE p.category IS NOT NULL
UNION ALL
SELECT CAST(s.uri AS TEXT) AS uri,
       CAST(s.title AS TEXT) AS title,
       CAST(trim(coalesce(p.description, '') || ' ' || coalesce(p.keywords, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('page_section') s
JOIN ref('page') p ON p.uri = s.page_uri
WHERE p.category IS NOT NULL
