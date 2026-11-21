-- Dependency manifests and lockfiles the change touched: what the code is
-- built from changed, which a diff of the code alone doesn't show. One row
-- per file and kind.
SELECT f.change_id, 'oxplow/review_signal_dependencies' AS source, 'dependencies' AS area,
       CASE r.kind WHEN 'lockfile' THEN 'lockfile-changed' ELSE 'dependency-manifest-changed' END AS signal,
       'high' AS severity, f.path AS path, NULL AS line, NULL AS end_line,
       CASE r.kind WHEN 'lockfile' THEN 'Lockfile changed' ELSE 'Dependency manifest changed' END AS title,
       printf('%s matches the %s rule %s, so the dependencies may have changed.',
              f.path,
              CASE r.kind WHEN 'lockfile' THEN 'lockfile' ELSE 'dependency manifest' END,
              min(r.glob)) AS why,
       NULL AS uri
FROM ref('change_file') f
JOIN ref('file_kind_rule') r ON r.kind IN ('dependency-manifest', 'lockfile') AND glob_match(r.glob, f.path)
GROUP BY f.change_id, f.path, r.kind
