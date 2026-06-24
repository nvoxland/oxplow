-- tsk269: run attribution unifies under a single ledger kind "run" (was the
-- per-producer "test-run"). All agent-work runs (tests/coverage/analysis) now
-- ride one kind, observed by trigger='on-report'. Rename existing rows so open
-- efforts at upgrade keep their claims.
UPDATE effort_attribution SET kind = 'run' WHERE kind = 'test-run';
