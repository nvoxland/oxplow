-- Every extension's review signals with the project's severity for each
-- kind (`review.signals`; the last row for a kind wins) and without the
-- kinds it turned off.
WITH setting AS (
  SELECT json_extract(value, '$.signal') AS signal, json_extract(value, '$.severity') AS severity, max(key)
  FROM json_each(config('/review/signals'))
  GROUP BY 1
)
-- The union's column types follow its contributors (a new source can move
-- them), so every column but `change_id` is cast to a fixed type here.
-- `change_id` stays as it is: a filter on it reaches each source.
SELECT s.change_id, CAST(s.source AS TEXT) AS source, CAST(s.area AS TEXT) AS area,
       CAST(s.signal AS TEXT) AS signal, CAST(coalesce(setting.severity, s.severity) AS TEXT) AS severity,
       CAST(s.path AS TEXT) AS path, CAST(s.line AS INTEGER) AS line, CAST(s.end_line AS INTEGER) AS end_line,
       CAST(s.title AS TEXT) AS title, CAST(s.why AS TEXT) AS why, CAST(s.uri AS TEXT) AS uri,
       CAST(s.severity AS TEXT) AS source_severity
FROM ref('review_signal') s
LEFT JOIN setting ON setting.signal = s.signal
WHERE coalesce(setting.severity, s.severity) <> 'off'
