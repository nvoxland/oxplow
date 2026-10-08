SELECT f.id, f.capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, s.kind AS subject_kind, s.ref AS subject_ref, p.path, f.line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
FROM source('fact') f
JOIN source('measure') m ON m.id = f.measure_id
JOIN source('metric_capture') c ON c.id = f.capture_id
LEFT JOIN source('fact_subject') s ON s.id = f.subject_id
LEFT JOIN source('fact_path') p ON p.id = f.path_id
LEFT JOIN source('fact_dims') d ON d.id = f.dims_id
