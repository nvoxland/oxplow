-- tsk73 — token-usage economics, in TOKENS (never dollars: the API returns
-- token counts only, and a locally maintained price table is invalid by
-- construction — see .context/metrics.md).
--
-- Three measures, deliberately SEPARATE from `oxplow.tokens`:
-- `agent.tokens.total` is an unfiltered sum over `oxplow.tokens`, so emitting
-- cache kinds there would silently change its meaning. And cross-time collapse
-- is per-MEASURE (`range_value`): a token SUM (additive) and a hit RATIO
-- (non-additive Σn/Σd) cannot share one measure.
--
-- - `oxplow.cache_tokens`  — prompt-cache token counts from the OTLP exports,
--   sliced by the existing `oxplow.token_kind` dimension with the new values
--   `cache_read` / `cache_creation`. Additive events, like `oxplow.tokens`.
--   Read by `agent.tokens.cache_read` / `agent.tokens.cache_creation` (sums).
-- - `oxplow.cache_usage`   — one ratio fact per OTLP export per model:
--   num = cache_read, den = input + cache_read + cache_creation (prompt-side;
--   output can't be cached). NON-ADDITIVE so the collapse is the cumulative
--   Σn/Σd hit ratio. Read by `agent.tokens.cache_hit_pct` (ratio, %).
-- - `oxplow.effort_tokens` — one fact per closed effort: the total tokens the
--   effort spent (all four kinds, from its effort-stamped otel captures).
--   NON-ADDITIVE with denominator 1, like the sibling per-close lifecycle
--   measures (`oxplow.cycle_time`, V47/V53): the collapse is the MEAN per
--   closed effort. Read by `task.tokens` (avg tokens per closed effort).

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.cache_tokens', 'Prompt-cache tokens', 'count', 'model', 'additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.cache_usage', 'Prompt-cache hit ratio', '%', 'model', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.effort_tokens', 'Effort token spend', 'count', 'effort', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
