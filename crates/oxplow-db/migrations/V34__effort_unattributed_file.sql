-- Unattributed/unreviewed effort changes (claim-first attribution, Child 2).
--
-- When an effort closes by a route that can't nudge a live agent (an
-- out-of-band IPC close, a death/restart), the claimed-vs-changed
-- reconciliation records the `changed_but_not_claimed` delta here instead of
-- leaving it only in the raw snapshot diff — where it could be mistaken for
-- the agent's authored work. This is the AUDIT residue: the snapshot saw
-- these paths change during the effort window, but nothing (no structured
-- auto-claim, no `touched_files`) claimed them.
--
-- Invariant: a path is either CLAIMED (`task_effort_file`) or UNATTRIBUTED
-- here, never both. Claiming a path (`record_file`) deletes any matching
-- row here, so a later `complete_task` that claims a previously-unattributed
-- path moves it back into the claimed set.
--
-- CASCADE with the effort like `task_effort_file`.

CREATE TABLE effort_unattributed_file (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, path)
);
