SELECT f.id, f.scan_id, s.tool, f.path, f.start_line, f.end_line, f.kind,
       f.metric_value, f.extra_json, s.started_at AS scanned_at
FROM source('code_quality_finding') f
JOIN source('code_quality_scan') s ON s.id = f.scan_id
