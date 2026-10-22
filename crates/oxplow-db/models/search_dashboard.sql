-- What site search indexes for each dashboard: its title, and in the body
-- the heading text of its text tiles and the titles of its query tiles.
-- Dashboards belong to the project, not a stream. `version` is the newer of
-- the dashboard's change and its tiles'.
SELECT CAST(d.uri AS TEXT) AS uri,
       CAST(d.title AS TEXT) AS title,
       CAST(trim(coalesce((SELECT group_concat(char(10) || t.text, '')
                             FROM (SELECT CASE kind
                                            WHEN 'text' THEN json_extract(options_json, '$.text')
                                            ELSE json_extract(options_json, '$.title') END AS text
                                     FROM source('dashboard_item')
                                    WHERE dashboard_id = d.id ORDER BY sort_index) t
                            WHERE t.text IS NOT NULL AND t.text <> ''), '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id,
       CAST(max(
         d.updated_at,
         coalesce((SELECT max(updated_at) FROM source('dashboard_item')
                    WHERE dashboard_id = d.id), '')
       ) AS TEXT) AS version
FROM ref('dashboard') d
