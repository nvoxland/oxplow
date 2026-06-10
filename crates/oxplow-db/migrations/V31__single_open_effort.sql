-- Enforce the lifecycle invariant: a task has at most ONE open effort
-- row (`ended_at IS NULL`) at a time. The lifecycle code always meant
-- this (update() opens on in_progress entry, finishes on exit;
-- record_effort merges into the open row), but nothing enforced it —
-- a skipped entry-effort or a crash between writes could accumulate
-- divergent open rows that silently mis-route attribution.
--
-- Heal first: close every open effort except the newest per task,
-- stamping ended_at with the row's own started_at (the only timestamp
-- we know is well-formed for the row). The newest open row survives
-- as the live one.

UPDATE task_effort
   SET ended_at = started_at
 WHERE ended_at IS NULL
   AND id NOT IN (
        SELECT MAX(id) FROM task_effort WHERE ended_at IS NULL GROUP BY task_id
   );

-- From here on, opening a second effort for the same task is a
-- constraint violation (surfaced as DomainError::Constraint).
CREATE UNIQUE INDEX idx_task_effort_open_unique
    ON task_effort(task_id)
 WHERE ended_at IS NULL;
