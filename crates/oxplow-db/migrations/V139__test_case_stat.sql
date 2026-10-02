-- P7 review (tsk733): one row per test per (stream, branch, producer) — what
-- each test is doing now and how it has behaved — updated in the same
-- transaction as each test run. It carries the reports (flaky, slowest,
-- recently broken) so a run records per-case facts only where they say
-- something new: every failure, and a pass or skip only when the test is new
-- on the branch, changed status, or moved its duration past the tolerance
-- (`recorded_ms` is the last duration written as a fact, which the tolerance
-- compares against so slow drift still surfaces).
CREATE TABLE test_case_stat (
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    producer TEXT NOT NULL,
    subject TEXT NOT NULL,
    last_status TEXT NOT NULL,
    last_ms REAL,
    recorded_ms REAL,
    max_ms REAL,
    timed_runs INTEGER NOT NULL DEFAULT 0,
    total_ms REAL NOT NULL DEFAULT 0,
    runs INTEGER NOT NULL,
    failures INTEGER NOT NULL,
    flips INTEGER NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    last_failed_at TEXT,
    last_passed_at TEXT,
    last_run_id INTEGER REFERENCES metric_capture(id) ON DELETE SET NULL,
    PRIMARY KEY (stream_id, branch, producer, subject)
) STRICT, WITHOUT ROWID;
