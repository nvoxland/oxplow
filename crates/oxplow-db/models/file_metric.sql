-- Every per-file code fact in the current tree (per stream): one row per
-- file and measure — TODO counts, doc coverage, and any per-path measure
-- a gauge reports per file.
SELECT t.stream_id, t.path, t.measure_key, t.value, t.numerator,
       t.denominator, t.captured_at
  FROM ref('tree_fact') t
 WHERE t.subject_kind = 'file'
