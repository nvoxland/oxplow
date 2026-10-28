SELECT effort_id, key, title, unit, direction, kind, category, language, agg,
       baseline, current, delta, changed, attributed_files, sample_count,
       target, warn_at, fail_at, crossing, latest_capture_id, refreshed_at
FROM source('effort_metric_delta')
