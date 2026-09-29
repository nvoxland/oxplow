SELECT f.id, f.capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, f.subject_kind, f.subject_ref, f.path, f.line,
       f.severity, f.rule, f.detail, f.dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
FROM source('fact') f
JOIN source('measure') m ON m.id = f.measure_id
JOIN source('metric_capture') c ON c.id = f.capture_id
