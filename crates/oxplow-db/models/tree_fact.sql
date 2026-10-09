-- The current facts of every `per-path` measure, per stream: the same
-- incremental-tree fold the metric engine reads (`latest_tree_facts`; an
-- equivalence test holds them together). A code gauge's capture restates
-- only the paths in its snapshot, so the tree is, per (stream, producer,
-- path), the facts from the latest capture that restated that path — a
-- delta capture its snapshot's rows, a full one the tree reconstructed as
-- of its snapshot, an asserted one the paths it emitted. A deleted file's
-- tombstone drops it; a removed function goes with its file's rescan.
-- "The facts from" a capture are the ones it holds for the path: its own
-- rows, or through its hold (`fact_path_hold`, V43) at the lines it saw.
WITH rel AS (
  SELECT DISTINCT c2.producer AS producer
    FROM source('fact') f2
    JOIN source('metric_capture') c2 ON c2.id = f2.capture_id
    JOIN source('measure') m2 ON m2.id = f2.measure_id
   WHERE m2.capture_scope = 'per-path'
),
anchor_tree AS (
  SELECT stream_id, anchor, path, storage FROM (
    SELECT a.stream_id AS stream_id, a.snapshot_id AS anchor,
           fs.path AS path, fs.storage AS storage,
           ROW_NUMBER() OVER (
             PARTITION BY a.stream_id, a.snapshot_id, fs.path
             ORDER BY fs.snapshot_id DESC, fs.id DESC
           ) AS rn
      FROM (SELECT DISTINCT stream_id, snapshot_id
              FROM source('metric_capture')
             WHERE scan_kind = 'full' AND status = 'done'
               AND snapshot_id IS NOT NULL
               AND producer IN (SELECT producer FROM rel)) a
      JOIN source('file_snapshot') fs
        ON fs.stream_id = a.stream_id
       AND fs.snapshot_id IS NOT NULL
       AND fs.snapshot_id <= a.snapshot_id
  ) WHERE rn = 1
),
restated AS (
  SELECT c.id AS capture_id, c.stream_id AS stream_id,
         c.producer AS producer, c.captured_at AS captured_at,
         fs.path AS path, fs.storage AS storage
    FROM source('metric_capture') c
    JOIN source('file_snapshot') fs
      ON fs.snapshot_id = c.snapshot_id
     AND fs.stream_id = c.stream_id
   WHERE c.snapshot_id IS NOT NULL AND c.status = 'done'
     AND c.scan_kind = 'delta'
     AND c.producer IN (SELECT producer FROM rel)
  UNION
  SELECT c.id, c.stream_id, c.producer, c.captured_at, t.path, t.storage
    FROM source('metric_capture') c
    JOIN anchor_tree t
      ON t.stream_id = c.stream_id AND t.anchor = c.snapshot_id
   WHERE c.scan_kind = 'full' AND c.status = 'done'
     AND c.producer IN (SELECT producer FROM rel)
     AND NOT EXISTS (
       SELECT 1 FROM source('metric_capture') c3
        WHERE c3.stream_id = c.stream_id
          AND c3.producer = c.producer
          AND c3.scan_kind = 'full' AND c3.status = 'done'
          AND (c3.captured_at > c.captured_at
               OR (c3.captured_at = c.captured_at AND c3.id > c.id))
     )
  UNION
  SELECT c.id, c.stream_id, c.producer, c.captured_at, fp.path, 'oxplow'
    FROM source('metric_capture') c
    JOIN source('fact') f ON f.capture_id = c.id
    JOIN source('fact_path') fp ON fp.id = f.path_id
   WHERE c.scan_kind = 'asserted'
     AND c.status = 'done'
     AND c.producer IN (SELECT producer FROM rel)
),
ranked AS (
  SELECT capture_id, path, storage,
         ROW_NUMBER() OVER (
           PARTITION BY stream_id, producer, path
           ORDER BY captured_at DESC, capture_id DESC
         ) AS rn
    FROM restated
),
winners AS (
  SELECT s.capture_id, fp.id AS path_id, fp.path AS path
    FROM ranked s JOIN source('fact_path') fp ON fp.path = s.path
   WHERE s.rn = 1 AND s.storage <> 'deleted'
)
SELECT f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, subj.kind AS subject_kind, subj.ref AS subject_ref, w.path, f.line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
  FROM winners w
  CROSS JOIN source('fact') f ON f.capture_id = w.capture_id AND f.path_id = w.path_id
  CROSS JOIN source('metric_capture') c ON c.id = f.capture_id
  JOIN source('measure') m ON m.id = f.measure_id
  LEFT JOIN source('fact_subject') subj ON subj.id = f.subject_id
  LEFT JOIN source('fact_dims') d ON d.id = f.dims_id
 WHERE m.capture_scope = 'per-path'
   AND NOT EXISTS (SELECT 1 FROM source('fact_path_hold') y
                    WHERE y.capture_id = c.id AND y.measure_id = f.measure_id
                      AND y.path_id = f.path_id)
UNION ALL
SELECT f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, subj.kind AS subject_kind, subj.ref AS subject_ref, w.path,
       -- CAST keeps the column INTEGER across the UNION.
       CAST(coalesce((SELECT fl.line FROM source('fact_line') fl
                       WHERE fl.fact_id = f.id AND fl.capture_id <= ph.capture_id
                       ORDER BY fl.capture_id DESC LIMIT 1), f.line) AS INTEGER) AS line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
  FROM winners w
  CROSS JOIN source('fact_path_hold') ph ON ph.capture_id = w.capture_id AND ph.path_id = w.path_id
  CROSS JOIN source('metric_capture') c ON c.id = ph.capture_id
  CROSS JOIN source('fact') f INDEXED BY idx_fact_measure_path ON f.measure_id = ph.measure_id AND f.path_id = ph.path_id
                      AND f.capture_id >= ph.from_capture_id AND f.capture_id <= ph.capture_id
                      AND (f.last_capture_id IS NULL OR f.last_capture_id >= ph.capture_id)
  CROSS JOIN source('metric_capture') fc ON fc.id = f.capture_id
                                  AND fc.stream_id = c.stream_id AND fc.producer = c.producer
                                  AND fc.status = 'done'
  JOIN source('measure') m ON m.id = f.measure_id
  LEFT JOIN source('fact_subject') subj ON subj.id = f.subject_id
  LEFT JOIN source('fact_dims') d ON d.id = f.dims_id
 WHERE m.capture_scope = 'per-path'
