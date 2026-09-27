-- Per changed file, how much its tests check before and after the change
-- (tsk314): test functions, assertions and skip markers. Read by the
-- oxplow-review Tests Weakened lens.
CREATE TABLE change_test_file (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    tests_before INTEGER NOT NULL,
    tests_after INTEGER NOT NULL,
    assertions_before INTEGER NOT NULL,
    assertions_after INTEGER NOT NULL,
    skips_before INTEGER NOT NULL,
    skips_after INTEGER NOT NULL,
    PRIMARY KEY (change_id, path)
);

CREATE VIEW v_change_test_file AS
SELECT change_id, path, tests_before, tests_after, assertions_before,
       assertions_after, skips_before, skips_after
FROM change_test_file;
