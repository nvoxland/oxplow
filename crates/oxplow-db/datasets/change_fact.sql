-- The per-path facts of each analysed change's changed files as of the
-- change's head: `v_tree_fact`'s fold (the latest capture that restated a
-- path, per stream, producer and path; a tombstone drops it) bounded to the
-- captures at or before the head's snapshot, and only for the files the
-- change changed. The head is `snap:<id>`, or `working` (the stream's latest
-- snapshot); a `git:<sha>` head has no snapshot to bound by and gets no rows.
--
-- Cheap by construction: it starts from (change, changed path), finds the
-- producers that have ever recorded a fact on that path, and picks each
-- one's winning capture with one indexed probe over the path's own history
-- (`file_snapshot` rows of the path), never over every capture.
WITH heads AS (
  SELECT ch.id AS change_id, ch.stream_id AS stream_id,
         CASE WHEN ch.head_revision LIKE 'snap:%' THEN CAST(substr(ch.head_revision, 6) AS INTEGER)
              WHEN ch.head_revision = 'working'
                THEN coalesce((SELECT max(s.id) FROM source('snapshot') s WHERE s.stream_id = ch.stream_id), 0)
         END AS snap,
         -- Asserted facts (no snapshot) at the head's time; a working head is now.
         CASE WHEN ch.head_revision LIKE 'snap:%'
              THEN (SELECT s.created_at FROM source('snapshot') s
                     WHERE s.id = CAST(substr(ch.head_revision, 6) AS INTEGER))
         END AS bound
    FROM source('change') ch
   WHERE ch.status = 'done'
),
keys AS (
  SELECT h.change_id, h.stream_id, h.snap, h.bound, cf.path AS path, fp.id AS path_id, pr.producer AS producer
    FROM heads h
    JOIN source('change_file') cf ON cf.change_id = h.change_id
    JOIN source('fact_path') fp ON fp.path = cf.path
    CROSS JOIN source('measure') m
    CROSS JOIN source('fact') pf ON pf.measure_id = m.id AND pf.path_id = fp.id
    CROSS JOIN source('metric_capture') pr ON pr.id = pf.capture_id AND pr.stream_id = h.stream_id
   WHERE h.snap IS NOT NULL AND m.capture_scope = 'per-path'
   GROUP BY h.change_id, cf.path, pr.producer
),
picked AS (
  SELECT k.change_id, k.stream_id, k.path_id, k.path,
         (
           SELECT cand.packed FROM (
             -- A delta capture restates the paths of its own snapshot.
             SELECT c.captured_at || '|' || printf('%012d', c.id) || '|' || fs.storage AS packed
               FROM source('file_snapshot') fs
               JOIN source('metric_capture') c
                 ON c.snapshot_id = fs.snapshot_id AND c.stream_id = fs.stream_id
              WHERE fs.stream_id = k.stream_id AND fs.path = k.path
                AND fs.snapshot_id IS NOT NULL AND fs.snapshot_id <= k.snap
                AND c.producer = k.producer AND c.status = 'done' AND c.scan_kind = 'delta'
                AND NOT EXISTS (SELECT 1 FROM source('metric_capture_exclusion') x
                                 WHERE x.capture_id = c.id AND x.path = k.path)
             UNION ALL
             -- The latest full capture at or before the head restates the
             -- tree reconstructed as of its own snapshot.
             SELECT c.captured_at || '|' || printf('%012d', c.id) || '|' || t.storage
               FROM source('metric_capture') c
               JOIN (SELECT fs2.stream_id, fs2.snapshot_id AS up_to, fs2.storage AS storage
                       FROM source('file_snapshot') fs2
                      WHERE fs2.stream_id = k.stream_id AND fs2.path = k.path
                        AND fs2.snapshot_id IS NOT NULL AND fs2.snapshot_id <= k.snap) t
                 ON t.stream_id = c.stream_id AND t.up_to <= c.snapshot_id
              WHERE c.stream_id = k.stream_id AND c.producer = k.producer
                AND c.scan_kind = 'full' AND c.status = 'done'
                AND c.snapshot_id IS NOT NULL AND c.snapshot_id <= k.snap
                AND NOT EXISTS (SELECT 1 FROM source('metric_capture_exclusion') x
                                 WHERE x.capture_id = c.id AND x.path = k.path)
                AND NOT EXISTS (
                  SELECT 1 FROM source('metric_capture') c3
                   WHERE c3.stream_id = c.stream_id AND c3.producer = c.producer
                     AND c3.scan_kind = 'full' AND c3.status = 'done'
                     AND c3.snapshot_id IS NOT NULL AND c3.snapshot_id <= k.snap
                     AND (c3.captured_at > c.captured_at
                          OR (c3.captured_at = c.captured_at AND c3.id > c.id)))
                -- The path's row as of the capture's snapshot, not a later one.
                AND t.up_to = (SELECT max(fs3.snapshot_id) FROM source('file_snapshot') fs3
                                WHERE fs3.stream_id = c.stream_id AND fs3.path = k.path
                                  AND fs3.snapshot_id IS NOT NULL AND fs3.snapshot_id <= c.snapshot_id)
             UNION ALL
             -- An asserted capture restates the paths it emitted facts for.
             SELECT c.captured_at || '|' || printf('%012d', c.id) || '|oxplow'
               FROM source('fact') af
               JOIN source('metric_capture') c ON c.id = af.capture_id
              WHERE af.path_id = k.path_id AND c.stream_id = k.stream_id
                AND c.producer = k.producer AND c.scan_kind = 'asserted' AND c.status = 'done'
                AND (k.bound IS NULL OR c.captured_at <= k.bound)
           ) cand
           ORDER BY cand.packed DESC LIMIT 1
         ) AS packed
    FROM keys k
),
winners AS (
  SELECT p.change_id, p.path_id, p.path,
         CAST(substr(p.packed, instr(p.packed, '|') + 1, 12) AS INTEGER) AS capture_id
    FROM picked p
   WHERE p.packed IS NOT NULL AND substr(p.packed, instr(p.packed, '|') + 14) <> 'deleted'
)
-- The facts the winning capture holds for the path: its own rows, or through
-- its hold (`fact_path_hold`, V43) at the lines it saw.
SELECT w.change_id AS change_id, f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, subj.kind AS subject_kind, subj.uri AS subject, w.path, f.line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.turn_id, c.captured_at, c.branch
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
SELECT w.change_id, f.id, c.id AS capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, subj.kind AS subject_kind, subj.uri AS subject, w.path,
       CAST(coalesce((SELECT fl.line FROM source('fact_line') fl
                       WHERE fl.fact_id = f.id AND fl.capture_id <= ph.capture_id
                       ORDER BY fl.capture_id DESC LIMIT 1), f.line) AS INTEGER) AS line,
       f.severity, f.rule, f.detail, d.json AS dims_json,
       c.stream_id, c.thread_id, c.turn_id, c.captured_at, c.branch
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
