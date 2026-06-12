-- V30 added threads.agent with CHECK (agent IN ('claude', 'codex'));
-- opencode is now a third supported agent. SQLite can't alter a CHECK
-- in place, and a full table rebuild is off the table here: migrations
-- run on a connection with foreign_keys=ON (database.rs), where
-- `DROP TABLE threads` performs an implicit DELETE FROM and would
-- cascade-wipe every child row (task, agent_turn, comment, ...).
--
-- Swap the column instead — ADD a replacement with the widened CHECK,
-- copy values, DROP the old column (its column-level CHECK goes with
-- it), and RENAME back. RENAME COLUMN rewrites the CHECK to the final
-- name. No FK is touched; column order shifts but all reads are by
-- name.

ALTER TABLE threads ADD COLUMN agent_next TEXT NOT NULL DEFAULT 'claude'
    CHECK (agent_next IN ('claude', 'codex', 'opencode'));

UPDATE threads SET agent_next = agent;

ALTER TABLE threads DROP COLUMN agent;

ALTER TABLE threads RENAME COLUMN agent_next TO agent;
