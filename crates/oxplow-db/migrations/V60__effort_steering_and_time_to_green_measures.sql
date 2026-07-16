-- tsk76 — usage metrics phase 2 (the [producer] autonomy/velocity pair;
-- phase 1 = cache-hit ratio + tokens-per-effort, V59).
--
-- Two per-close lifecycle measures, same shape as `oxplow.cycle_time` /
-- `oxplow.effort_tokens` (V47/V59): one fact per closed effort, NON-ADDITIVE
-- with denominator 1, so the cross-time collapse is the MEAN across closes.
--
-- - `oxplow.effort_steering` — how many times a human (or oxplow on their
--   behalf) had to steer the effort: user prompt submissions (agent_turn rows
--   opened in the effort window) + Stop-hook nudges fired (the effort's
--   `oxplow.nudge` facts) + review comments opened in the effort's thread
--   during the window. Read by `task.steering` (avg per closed effort —
--   the autonomy number; lower = more autonomous). Interrupts are NOT
--   counted — nothing records them yet.
-- - `oxplow.effort_time_to_green` — wall-clock ms from the effort's FIRST red
--   test run to the first green run after it. Only emitted when that
--   red→green transition exists (an always-green or never-green effort is
--   "no data", not a zero). Read by `effort.time_to_green_ms` (avg).

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.effort_steering', 'Effort steering events', 'count', 'effort', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.effort_time_to_green', 'Effort time to green', 'ms', 'effort', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
