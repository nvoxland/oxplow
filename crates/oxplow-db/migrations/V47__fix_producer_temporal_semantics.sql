-- Fix producer measure temporal semantics (epic tsk12, review fix tsk42).
--
-- V43 seeded `oxplow.test_case` and `oxplow.lint_hit` as ADDITIVE, which made
-- headline_for_spec/range_value SUM per-run counts across every run ever —
-- "run a 100-test suite 10 times" read 1000. A test/analysis run REPLACES the
-- previous suite/lint state, so both are SEMI-ADDITIVE (last capture wins).
--
-- `oxplow.cycle_time` (additive) summed cycle times across all closed efforts
-- and `oxplow.task_effort` (semi-additive) collapsed to the last-closed task's
-- count — both specs want the MEAN across closes, and the cross-time collapse
-- has no "average", so they become NON-ADDITIVE: the producer writes each fact
-- with numerator = value, denominator = 1, and Σn/Σd across time is the mean.
UPDATE measure SET temporal_semantics = 'semi-additive'
    WHERE key IN ('oxplow.test_case', 'oxplow.lint_hit');
UPDATE measure SET temporal_semantics = 'non-additive'
    WHERE key IN ('oxplow.cycle_time', 'oxplow.task_effort');
