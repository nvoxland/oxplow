-- Kind-agnostic attribution ledger (epic tsk260, phase 2).
--
-- Generalizes the file claim-first tables (`task_effort_file` /
-- `effort_unattributed_file` / acknowledged) into one kind-discriminated table
-- so non-file fact-kinds — `test-run`, `coverage`, `analysis`, … — share the
-- claim → verify → reconcile engine. `FileKind` keeps its own existing tables
-- (no migration); every OTHER kind stores its claim/acknowledge/unattributed
-- state here.
--
-- One row per `(effort_id, kind, ref)`; `state` holds the single attribution
-- state — the same "a ref is in exactly one of {claimed, unattributed,
-- acknowledged}, never two" invariant the file tables enforce structurally.
-- `ref` is the kind's item identity (e.g. a `metric_run` id for `test-run`);
-- `detail_json` is optional kind-specific context. CASCADE with the effort like
-- `task_effort_file`, so GC of the effort drops its ledger rows.

CREATE TABLE effort_attribution (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('claimed', 'unattributed', 'acknowledged')),
    detail_json TEXT,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, kind, ref)
);

-- Cross-effort dedup ("a ref another effort already claimed isn't this agent's
-- to flag") + per-kind state reads.
CREATE INDEX idx_effort_attribution_kind_state
    ON effort_attribution(kind, state);
