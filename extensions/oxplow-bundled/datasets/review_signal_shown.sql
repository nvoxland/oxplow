-- Every extension's review signals with the project's severity for each
-- kind (`review.signals`; the last row for a kind wins) and without the
-- kinds it turned off.
WITH setting AS (
  SELECT json_extract(value, '$.signal') AS signal, json_extract(value, '$.severity') AS severity, max(key)
  FROM json_each(config('/review/signals'))
  GROUP BY 1
)
SELECT s.change_id, s.source, s.area, s.signal, coalesce(setting.severity, s.severity) AS severity,
       s.path, s.line, s.end_line, s.title, s.why, s.uri, s.severity AS source_severity
FROM ref('review_signal') s
LEFT JOIN setting ON setting.signal = s.signal
WHERE coalesce(setting.severity, s.severity) <> 'off'
