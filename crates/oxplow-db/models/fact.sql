-- Every fact each capture holds. A complete-scope capture stores only what
-- changed since its producer's previous scan and holds the facts it
-- repeats (`fact_chain`, V39): a fact appears once per capture holding it,
-- with that capture's columns. The same rule as the engine's reads
-- (`fact_chain::held_facts_sql`).
SELECT f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, s.kind AS subject_kind, s.ref AS subject_ref, p.path, f.line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
FROM source('fact') f
JOIN source('measure') m ON m.id = f.measure_id
JOIN source('metric_capture') c ON c.id = f.capture_id
LEFT JOIN source('fact_subject') s ON s.id = f.subject_id
LEFT JOIN source('fact_path') p ON p.id = f.path_id
LEFT JOIN source('fact_dims') d ON d.id = f.dims_id
WHERE NOT EXISTS (SELECT 1 FROM source('fact_chain') x
                   WHERE x.capture_id = c.id AND x.measure_id = f.measure_id)
UNION ALL
SELECT f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, s.kind AS subject_kind, s.ref AS subject_ref, p.path, f.line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
FROM source('fact_chain') ch
-- CROSS JOIN pins the order: from the chain rows, never from every fact.
CROSS JOIN source('metric_capture') c ON c.id = ch.capture_id
CROSS JOIN source('fact') f ON f.measure_id = ch.measure_id
                    AND f.capture_id >= ch.from_capture_id AND f.capture_id <= ch.capture_id
                    AND (f.last_capture_id IS NULL OR f.last_capture_id >= ch.capture_id)
CROSS JOIN source('metric_capture') fc ON fc.id = f.capture_id
                                AND fc.stream_id = c.stream_id AND fc.producer = c.producer
                                AND fc.status = 'done'
JOIN source('measure') m ON m.id = f.measure_id
LEFT JOIN source('fact_subject') s ON s.id = f.subject_id
LEFT JOIN source('fact_path') p ON p.id = f.path_id
LEFT JOIN source('fact_dims') d ON d.id = f.dims_id
