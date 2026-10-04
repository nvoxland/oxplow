-- P11 (tsk945) — a dimension key has one name: its namespaced, conformed one.
--
-- The engine used to read `oxplow.language` and bare `language` as each
-- other, and took bare `package` / `branch` / `model` requests for the
-- conformed ones. The aliases go; what was stored under a bare key takes the
-- conformed key here, and the fact ingest refuses an un-namespaced key from
-- now on.

-- Facts recorded before the collector scripts namespaced their dims carry
-- bare `language`. Where a fact has both, the conformed value is kept. The
-- `LIKE` comes first so only a row that could have the key is parsed.
UPDATE fact
   SET dims_json = CASE
         WHEN json_extract(dims_json, '$."oxplow.language"') IS NULL
           THEN json_set(json_remove(dims_json, '$.language'),
                         '$."oxplow.language"', json_extract(dims_json, '$.language'))
         ELSE json_remove(dims_json, '$.language')
       END
 WHERE dims_json LIKE '%"language"%'
   AND json_valid(dims_json)
   AND json_type(dims_json, '$.language') IS NOT NULL;

-- A nudge carried its kind twice: as its subject and as a bare `kind` dim
-- nothing read. The subject stays.
UPDATE fact
   SET dims_json = NULLIF(json_remove(dims_json, '$.kind'), '{}')
 WHERE measure_id IN (SELECT id FROM measure WHERE key = 'oxplow.nudge')
   AND dims_json LIKE '%"kind"%'
   AND json_valid(dims_json)
   AND json_type(dims_json, '$.kind') IS NOT NULL;

-- A spec's sliceable dims and its `dim_eq` filter take the conformed keys.
-- (`subject` is the raw-subject pseudo-dimension, not a catalog key.)
UPDATE metric_spec
   SET sliceable_dims_json = (
         SELECT json_group_array(
                  CASE WHEN d.value IN ('language', 'package', 'branch', 'model',
                                        'effort', 'thread', 'vcs_rev')
                       THEN 'oxplow.' || d.value ELSE d.value END)
           FROM json_each(metric_spec.sliceable_dims_json) d)
 WHERE json_valid(sliceable_dims_json)
   AND json_type(sliceable_dims_json) = 'array';

UPDATE metric_spec
   SET filter_json = json_set(filter_json, '$.dim_eq[0]',
                              'oxplow.' || json_extract(filter_json, '$.dim_eq[0]'))
 WHERE json_valid(filter_json)
   AND json_extract(filter_json, '$.dim_eq[0]') IN
         ('language', 'package', 'branch', 'model', 'effort', 'thread', 'vcs_rev');

-- The cube was folded over the old keys: what a replay computes changed, so
-- it is cleared and re-folds (the V64 / V66 rule).
DELETE FROM metric_cube;
DELETE FROM metric_live_fact;
DELETE FROM metric_cube_state;
