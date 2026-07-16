-- tsk77 — wasted-token ratio: tokens spent in efforts whose changes were
-- later REVERTED, as a share of all effort token spend. Token-denominated by
-- decision — never dollars (tsk73).
--
-- One measure, an append-only ratio scheme (the engine has no cross-measure
-- division, so both reads must fold from ONE measure):
--   - at effort CLOSE the lifecycle producer emits (value 0, num 0,
--     den = the effort's total tokens) — the effort enters the denominator.
--   - when a `git revert` of one of the effort's commits is detected, the
--     collection revert leg emits (value = the effort's tokens, num = same,
--     den 0) — the waste enters the numerator, idempotent per effort.
-- NON-ADDITIVE, so the cross-time collapse Σn/Σd = wasted/total (read by
-- `task.tokens.wasted_pct`, ratio ×100), while a plain SUM over values =
-- total wasted tokens (`task.tokens.wasted`) since close rows carry value 0.
-- Efforts closed before V61 never entered the denominator — the ratio only
-- describes V61-era closes.

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.token_waste', 'Reverted-effort token waste', 'count', 'effort', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
