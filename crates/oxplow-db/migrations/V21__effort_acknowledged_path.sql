-- Per-effort acknowledged unclaimed paths.
--
-- The auto-diff for an effort sometimes contains paths the agent did
-- not list under `touched_files` because they were edited by another
-- effort that bracketed the same snapshot window, or by a non-effort
-- actor (formatter, codegen, user). `amend_effort(remove_files=…)`
-- lets the agent disclaim such paths, but until now the disclaim only
-- removed any `task_effort_file` row for the path — there was no
-- record that the agent had explicitly *seen* the path and chosen not
-- to claim it. That left the Stop-hook file-review re-deriving the
-- same `changed_but_not_claimed` discrepancy on every recompute,
-- forcing one more directive fire before silent-agreement kicks in.
--
-- This table records the acknowledgement durably. The Stop hook's
-- `recompute_effort_file_review` subtracts acknowledged paths from
-- the diff before deciding whether the discrepancy still warrants
-- prompting the agent.

CREATE TABLE effort_acknowledged_path (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    acknowledged_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (effort_id, path)
);
