-- "Look here first": each changed file's review priority and why. Size,
-- complexity spikes, parameter growth and long new functions combine
-- multiplicatively, so one hot factor dominates:
--   (1 + log2(1 + added + deleted))
--   × (1 + 0.6 · complexity added in changed bodies)
--   × (1 + 0.4 · parameters added to changed signatures)
--   × (1 + (longest new function over 60 lines − 60) / 40)
-- A factor of at least 1.2 (and 32+ lines touched) gives a reason.
WITH fns AS (
  SELECT change_id, path,
         sum(CASE WHEN body_changed = 1 AND complexity_delta > 0 THEN complexity_delta ELSE 0 END) AS spike,
         sum(CASE WHEN body_changed = 1 AND complexity_delta > 0 THEN 1 ELSE 0 END) AS spiked,
         sum(CASE WHEN signature_changed = 1 AND params_after - params_before > 0
                  THEN params_after - params_before ELSE 0 END) AS grown_by,
         sum(CASE WHEN signature_changed = 1 AND params_after - params_before > 0 THEN 1 ELSE 0 END) AS grown,
         max(CASE WHEN status = 'added' AND length > 60 THEN length END) AS longest_new
  FROM ref('change_function')
  GROUP BY change_id, path
),
factors AS (
  SELECT f.change_id, f.path, f.additions + f.deletions AS touched,
         log2(1 + f.additions + f.deletions) AS size,
         coalesce(n.spike, 0) AS spike, coalesce(n.spiked, 0) AS spiked,
         coalesce(n.grown_by, 0) AS grown_by, coalesce(n.grown, 0) AS grown,
         n.longest_new,
         1 + 0.6 * coalesce(n.spike, 0) AS complexity_factor,
         1 + 0.4 * coalesce(n.grown_by, 0) AS param_factor,
         1 + coalesce((n.longest_new - 60) / 40.0, 0) AS long_factor
  FROM ref('change_file') f
  LEFT JOIN fns n ON n.change_id = f.change_id AND n.path = f.path
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
FROM factors
