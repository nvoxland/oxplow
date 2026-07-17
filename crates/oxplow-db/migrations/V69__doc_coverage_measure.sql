-- Doc coverage (tsk125) — % of PUBLIC functions/methods that carry a doc
-- comment (or, for Python/Clojure, a docstring). An observed completeness ratio
-- oxplow computes itself from the AST (no external tool), via the language-
-- agnostic `oxplow.doc_coverage` code gauge over the `code_metrics()` capability
-- (which now reports `has_doc`).
--
-- One per-file ratio fact: numerator = documented public items, denominator =
-- public items, value = %. `semi-additive` (a scan restates the value) and
-- `per-path` (a snapshot gauge restates only the files in its snapshot — the
-- delta model the other code-gauge measures use). A new measure row is a catalog
-- INSERT, not a table rebuild, so no fact CASCADE fires.

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, capture_scope, component_role, created_at, updated_at) VALUES
    ('oxplow.doc_coverage', 'Doc coverage', '%', 'file', 'semi-additive', 'per-path', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
