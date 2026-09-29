SELECT key, title, unit, source_measure, aggregation, direction, target,
       warn_at, fail_at, description, category, language,
       CASE WHEN extension IS NOT NULL THEN 'extension' ELSE scope END AS scope,
       display_kind, extension
FROM source('metric_spec')
