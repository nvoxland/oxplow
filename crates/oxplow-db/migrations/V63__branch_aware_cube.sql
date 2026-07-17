-- tsk97 — key the cube's live state by BRANCH, mirroring the fact fold.
--
-- 50fd1760 made the fact fold's state per (stream, branch): a capture folds
-- only into its own branch's tree, and a new branch SEEDS from the history
-- visible at its first capture, so it inherits the pre-fork suite instead of
-- collapsing to what it re-ran. The cube's build kept a branch-blind state, so
-- a multi-branch measure's stored rows encoded a fold the fact path no longer
-- computes — feature-x's failure landing on a point labelled main.
-- `cube_series` declined those measures rather than serve the divergence
-- (0865a39a); this migration is what lets the build catch up and that guard
-- come out.
--
-- Both tables are part of the DISPOSABLE cube (see V62): dropping and
-- recreating them costs a one-time backfill, never data. `metric_cube` itself
-- needs NO branch column — its grain is the capture and the capture carries
-- the branch — but its rows are cleared so nothing computed by the
-- branch-blind build survives to be served.
--
-- `branch` is TEXT NOT NULL with '' meaning "the capture carries no branch":
-- a WITHOUT ROWID PK cannot hold NULL, and no-branch is a real partition (the
-- fold keys on `Option<String>`, where `None` is its own branch), not an
-- absence to coalesce away. The '' mapping lives in the store layer only.

DROP TABLE metric_live_fact;
DROP TABLE metric_cube_state;
DELETE FROM metric_cube;

-- See V62 for what this table IS (the fold's live state made durable, and why
-- re-aggregating it beats delta arithmetic — min/max are not decrementable).
-- V63 adds `branch` to the key: a capture may only evict/insert within its own
-- branch's partition, else a feature branch's re-run rewrites main's points
-- whenever main didn't re-run that subject.
CREATE TABLE metric_live_fact (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL,
    branch TEXT NOT NULL,
    producer TEXT NOT NULL,
    subject_key TEXT NOT NULL,
    fact_id INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
    PRIMARY KEY (measure_id, stream_id, branch, producer, subject_key, fact_id)
) WITHOUT ROWID;

-- How far the cube has been built, now per (measure, stream, BRANCH). A row
-- does double duty:
-- - its (captured_at, id) is that branch's watermark, and because the build
--   processes a stream's captures in global (captured_at, id) order, the
--   STREAM's watermark — what the read checks coverage against — is simply the
--   MAX across its branch rows;
-- - its existence is the "this branch has been seeded" marker. No row means
--   the branch's first capture hasn't been folded: the build replays the
--   visible history into the branch's live partition before applying it.
--   That marker is what keeps "seeded but legitimately empty" distinct from
--   "never seeded" — the same ambiguity the watermark already resolves for
--   cube rows (V62).
CREATE TABLE metric_cube_state (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL,
    branch TEXT NOT NULL,
    last_capture_id INTEGER NOT NULL,
    last_captured_at TEXT NOT NULL,
    PRIMARY KEY (measure_id, stream_id, branch)
) WITHOUT ROWID;
