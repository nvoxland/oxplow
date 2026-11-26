-- Every extension's review signals with the project's severity for each
-- kind (`review.signals`; the last row for a kind wins) and without the
-- kinds it turned off. A row with no severity (or an unknown one) is
-- `medium`.
WITH setting AS (
  SELECT json_extract(value, '$.signal') AS signal, json_extract(value, '$.severity') AS severity, max(key)
  FROM json_each(config('/review/signals'))
  GROUP BY 1
)
-- The union's column types follow its contributors (a new source can move
-- them), so every column but `change_id` is cast to a fixed type here.
-- `change_id` stays as it is: a filter on it reaches each source.
SELECT s.change_id, CAST(s.source AS TEXT) AS source, CAST(s.area AS TEXT) AS area,
       CAST(s.signal AS TEXT) AS signal,
       -- A source that gives no severity, or one the review doesn't know,
       -- is `medium`: its rows are shown, never lost.
       CAST(CASE WHEN coalesce(setting.severity, s.severity) IN ('high', 'medium', 'info')
                 THEN coalesce(setting.severity, s.severity) ELSE 'medium' END AS TEXT) AS severity,
       CAST(s.path AS TEXT) AS path, CAST(s.line AS INTEGER) AS line, CAST(s.end_line AS INTEGER) AS end_line,
       CAST(s.title AS TEXT) AS title, CAST(s.why AS TEXT) AS why, CAST(s.uri AS TEXT) AS uri,
       CAST(s.related AS TEXT) AS related,
       -- As a page shows them: its lines as one label, and five related
       -- places at most, the rest one item that names them on hover.
       CAST(CASE WHEN s.line IS NULL THEN ''
                 WHEN s.end_line IS NULL OR s.end_line = s.line THEN CAST(s.line AS TEXT)
                 ELSE printf('%d–%d', s.line, s.end_line) END AS TEXT) AS lines,
       CAST(CASE WHEN json_array_length(s.related) > 5 THEN
         json_insert((SELECT json_group_array(json(r.value)) FROM json_each(s.related) r WHERE r.key < 5),
           '$[#]', json_object('text', printf('%d more', json_array_length(s.related) - 5),
             'hint', (SELECT group_concat(json_extract(r.value, '$.text'), ', ') FROM json_each(s.related) r WHERE r.key >= 5)))
       ELSE coalesce(CAST(s.related AS TEXT), '[]') END AS TEXT) AS related_shown,
       CAST(s.severity AS TEXT) AS source_severity
FROM ref('review_signal') s
LEFT JOIN setting ON setting.signal = s.signal
WHERE coalesce(setting.severity, s.severity, '') <> 'off'
