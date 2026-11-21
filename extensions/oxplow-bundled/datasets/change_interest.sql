-- "Look here first": each changed file's review priority and why. Size,
-- complexity spikes, parameter growth and long new functions combine
-- multiplicatively, so one hot factor dominates:
--   (1 + log2(1 + added + deleted))
--   × (1 + 0.6 · complexity added in changed bodies)
--   × (1 + 0.4 · parameters added to changed signatures)
--   × (1 + (longest new function over 60 lines − 60) / 40)
-- (The 0.6, 0.4, 60 and 40 are this extension's `changeInterest` settings.)
-- A factor of at least 1.2 (and 32+ lines touched) gives a reason.
-- One aggregate over the file's own functions (a LEFT JOIN), so a filter on
-- `change_id` reaches the change's rows: a CTE joined to the files is
-- computed for every change first.
WITH factors AS (
  SELECT f.change_id, f.path, f.additions + f.deletions AS touched,
         log2(1 + f.additions + f.deletions) AS size,
         coalesce(sum(CASE WHEN fn.body_changed = 1 AND fn.complexity_delta > 0 THEN fn.complexity_delta ELSE 0 END), 0) AS spike,
         coalesce(sum(CASE WHEN fn.body_changed = 1 AND fn.complexity_delta > 0 THEN 1 ELSE 0 END), 0) AS spiked,
         coalesce(sum(CASE WHEN fn.signature_changed = 1 AND fn.params_after - fn.params_before > 0
                           THEN fn.params_after - fn.params_before ELSE 0 END), 0) AS grown_by,
         coalesce(sum(CASE WHEN fn.signature_changed = 1 AND fn.params_after - fn.params_before > 0 THEN 1 ELSE 0 END), 0) AS grown,
         max(CASE WHEN fn.status = 'added' AND fn.length > 60 THEN fn.length END) AS longest_new
  FROM ref('change_file') f
  LEFT JOIN ref('change_function') fn ON fn.change_id = f.change_id AND fn.path = f.path
  GROUP BY f.change_id, f.path, f.additions, f.deletions
),
scored AS (
  SELECT change_id, path, touched, size, spike, spiked, grown_by, grown, longest_new,
         1 + 0.6 * spike AS complexity_factor,
         1 + 0.4 * grown_by AS param_factor,
         1 + coalesce((longest_new - 60) / (1.0 * 40), 0) AS long_factor
  FROM factors
)
SELECT change_id, path,
       (1 + size) * complexity_factor * param_factor * long_factor AS interest,
       rtrim(
         CASE WHEN complexity_factor >= 1.2 THEN
           'complexity +' || CASE WHEN spike = CAST(spike AS INTEGER) THEN CAST(CAST(spike AS INTEGER) AS TEXT)
                                  ELSE printf('%.1f', spike) END
           || ' across ' || spiked || CASE WHEN spiked = 1 THEN ' fn' ELSE ' fns' END || '; '
         ELSE '' END
         || CASE WHEN param_factor >= 1.2 THEN
           '+' || grown_by || CASE WHEN grown_by = 1 THEN ' param' ELSE ' params' END
           || ' across ' || grown || CASE WHEN grown = 1 THEN ' fn' ELSE ' fns' END || '; '
         ELSE '' END
         || CASE WHEN longest_new IS NOT NULL AND long_factor >= 1.2 THEN
           'added ' || longest_new || '-line function; '
         ELSE '' END
         || CASE WHEN size >= 5 THEN touched || ' lines touched; ' ELSE '' END,
         '; ') AS reasons
FROM scored
