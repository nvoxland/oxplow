-- tsk102 — the metric-ancestry resolver replaces blind visibility, so every
-- cube seed built under blind() is now potentially wrong for multi-branch
-- streams (a blind seed inherits EVERYTHING earlier; a resolved seed excludes
-- siblings' absorbed work). Seeds are frozen into `metric_live_fact` at build
-- time, so the switch must clear the cube and let the backfill re-seed under
-- the resolver. Disposable by design (V62) — this costs one background
-- re-fold, never data.
DELETE FROM metric_cube;
DELETE FROM metric_live_fact;
DELETE FROM metric_cube_state;
