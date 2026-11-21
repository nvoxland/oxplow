-- Files in sensitive places (auth, secrets, crypto) and migrations the
-- change touched. A slip there costs more than elsewhere, so each is worth
-- a look (`medium`). One row per file and kind.
SELECT f.change_id, 'oxplow/review_signal_sensitive' AS source, 'code' AS area,
       CASE r.kind WHEN 'migration' THEN 'migration-changed' ELSE 'sensitive-path' END AS signal,
       'medium' AS severity, f.path AS path, NULL AS line, NULL AS end_line,
       CASE r.kind WHEN 'migration' THEN 'Migration changed' ELSE 'Sensitive file changed' END AS title,
       printf('%s matches the %s rule %s.',
              f.path,
              CASE r.kind WHEN 'migration' THEN 'migration' ELSE 'sensitive' END,
              min(r.glob)) AS why,
       NULL AS uri
FROM ref('change_file') f
JOIN ref('file_kind_rule') r ON r.kind IN ('sensitive', 'migration') AND glob_match(r.glob, f.path)
GROUP BY f.change_id, f.path, r.kind
