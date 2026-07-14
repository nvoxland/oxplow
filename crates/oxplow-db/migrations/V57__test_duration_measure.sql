-- tsk46 — per-test duration. Test time is a productivity tax; nothing tracked it,
-- even though the JUnit parser already extracts `timeMs` per case and threw it away.
--
-- `per-subject` (V55) is exactly the right scope: each test's LATEST duration wins, so
-- a PARTIAL run updates only the tests it actually ran while every other test keeps
-- its last-known timing. The sum therefore stays a real suite total instead of
-- collapsing to "the 4 tests I just ran" — the same trap `test_case` fell into.
--
-- Read by `oxplow.tests.duration_ms` (sum → total suite time) and
-- `oxplow.tests.slowest_ms` (max → the slowest single test).

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, capture_scope, component_role, created_at, updated_at) VALUES
    ('oxplow.test_duration', 'Test duration', 'ms', 'test', 'semi-additive', 'per-subject', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
