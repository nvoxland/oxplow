-- Epic tsk12 / child tsk30 (per-language idiom gauges emit facts). The bundled
-- per-language idiom gauges (unsafe_blocks, any_usage, empty_catch, …) invert to
-- facts on ONE generic measure — `oxplow.ast_hit` (an AST idiom occurrence,
-- per-file grain) — distinguished by the `oxplow.rule` dimension (the idiom slug,
-- read off the fact's `rule` column). Each such metric is a `Sum(oxplow.ast_hit)`
-- spec filtered by `dim_eq(oxplow.rule, <slug>)`, seeded from Rust
-- (`metrics_service.rs::builtin_ast_specs`).
--
-- Additive to V43's built-in catalog (keeps that migration append-only).

INSERT INTO dimension (key, label, value_type, subject_kind) VALUES
    ('oxplow.rule', 'Rule', 'categorical', NULL);

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.ast_hit', 'AST idiom hits', 'count', 'file', 'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
