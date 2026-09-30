-- One row per function in the current tree (per stream): its latest
-- complexity, length and parameter count from the code gauges' facts
-- (`symbol:<path>::<name>` subjects on the per-path code measures).
SELECT t.stream_id,
       t.subject_ref AS symbol,
       t.path,
       substr(t.subject_ref, instr(t.subject_ref, '::') + 2) AS name,
       min(t.line) AS line,
       max(json_extract(t.dims_json, '$."oxplow.language"')) AS language,
       max(CASE WHEN t.measure_key = 'oxplow.complexity' THEN t.value END) AS complexity,
       max(CASE WHEN t.measure_key = 'oxplow.fn_length' THEN t.value END) AS length,
       max(CASE WHEN t.measure_key = 'oxplow.parameter_count' THEN t.value END) AS parameters,
       max(t.captured_at) AS captured_at
  FROM ref('tree_fact') t
 WHERE t.subject_kind = 'symbol'
   AND t.measure_key IN ('oxplow.complexity', 'oxplow.fn_length', 'oxplow.parameter_count')
 GROUP BY t.stream_id, t.subject_ref, t.path
