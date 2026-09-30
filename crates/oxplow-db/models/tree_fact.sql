-- The current facts of every `per-path` measure, per stream: the same
-- incremental-tree fold the metric engine reads (`latest_tree_facts`; an
-- equivalence test holds them together). A code gauge's capture restates
-- only the paths in its snapshot, so the tree is, per (stream, producer,
-- path), the facts from the latest capture that restated that path — a
-- delta capture its snapshot's rows, a full one the tree reconstructed as
-- of its snapshot, an asserted one the paths it emitted. A deleted file's
-- tombstone drops it; a removed function goes with its file's rescan.
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
  SELECT c.id, c.stream_id, c.producer, c.captured_at, f.path, 'oxplow'
    FROM source('metric_capture') c
    JOIN source('fact') f ON f.capture_id = c.id
   WHERE c.scan_kind = 'asserted' AND f.path IS NOT NULL
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
)
SELECT f.id, f.capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, f.subject_kind, f.subject_ref, f.path, f.line,
       f.severity, f.rule, f.detail, f.dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
  FROM source('fact') f
  JOIN source('measure') m ON m.id = f.measure_id
  JOIN source('metric_capture') c ON c.id = f.capture_id
  JOIN ranked s ON s.capture_id = f.capture_id AND s.path = f.path
 WHERE m.capture_scope = 'per-path'
   AND s.rn = 1
   AND s.storage <> 'deleted'
