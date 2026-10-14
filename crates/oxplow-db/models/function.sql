-- One row per function in the current tree (per stream): its latest
-- complexity, length and parameter count from the code gauges (facts whose
-- subject is the function's `symbol:<path>/<id>` ref), named by the symbol
-- index (`v_symbol`) those refs come from.
SELECT t.stream_id,
       t.subject AS symbol,
       t.path,
       s.name,
       s.container,
       s.kind,
       min(t.line) AS line,
       max(json_extract(t.dims_json, '$."oxplow.language"')) AS language,
       max(CASE WHEN t.measure_key = 'oxplow.complexity' THEN t.value END) AS complexity,
       max(CASE WHEN t.measure_key = 'oxplow.fn_length' THEN t.value END) AS length,
       max(CASE WHEN t.measure_key = 'oxplow.parameter_count' THEN t.value END) AS parameters,
       max(t.captured_at) AS captured_at
  FROM ref('tree_fact') t
  JOIN ref('symbol') s ON s.stream_id = t.stream_id AND s.ref = t.subject
 WHERE t.subject_kind = 'symbol'
   AND t.measure_key IN ('oxplow.complexity', 'oxplow.fn_length', 'oxplow.parameter_count')
 GROUP BY t.stream_id, t.subject
