-- tsk101 — promote the four low-cardinality filter dims so 20 more specs cube.
--
-- tsk99's all-spec verification measured 42/68 specs cube-served; 20 of the 26
-- declines existed only because `cube_series` requires every filtered/grouped
-- dim to be in the cube's grain: 18 specs filter `dim_eq` on an unpromoted dim
-- (`oxplow.rule` ×10, `oxplow.token_kind` ×4, `oxplow.tests_stat` ×4) and 2
-- filter `severity`. Promoting those dims is the whole change — the read and
-- build already handle any promoted dim generically.
--
-- Cardinality, measured against the real DB before promoting (the same gate
-- that rejected `oxplow.test_suite` at 234 in V62): `oxplow.rule` 13,
-- `oxplow.token_kind` 4, `oxplow.tests_stat` 4, `oxplow.severity` 1. The dims
-- live on DIFFERENT measures' facts, so the grain does not cross-multiply —
-- a fact's `dims_key` carries only the promoted dims that fact actually has.
--
-- `oxplow.severity` and `oxplow.rule` are fact COLUMNS, not `dims_json` keys;
-- both routes flow through the same `dim_value`, which is what `dims_key`
-- buckets by — so promotion behaves identically for either storage.
--
-- Promoting a dim changes the GRAIN, and a grain change is a cube REBUILD
-- (never a schema change): clear the disposable cube and let the backfill
-- re-fold under the new grain. Old rows must not survive — a pre-promotion
-- bucket merged values the new grain separates, and serving it would answer a
-- newly-eligible filter with the wrong number.

UPDATE dimension SET promoted = 1
 WHERE key IN ('oxplow.rule', 'oxplow.token_kind', 'oxplow.tests_stat', 'oxplow.severity');

DELETE FROM metric_cube;
DELETE FROM metric_live_fact;
DELETE FROM metric_cube_state;
