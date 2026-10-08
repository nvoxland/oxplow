-- What a handler calls is a scope (`sql.read`; `.context/commands.md`
-- "Scopes"), no longer a "host capability" — a word kept for the swappable
-- pieces (`work_items`). The run's per-scope call counts follow the name:
-- `{"sql.read": 2}`, NULL when it called none.
ALTER TABLE command_audit RENAME COLUMN capabilities_json TO scopes_json;
