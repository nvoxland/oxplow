-- What a run used of the host capabilities (`.context/commands.md` "Host
-- capabilities"): per capability id, how many calls — `{"sql.read": 2}`.
-- NULL when it used none (every run before this).
ALTER TABLE command_audit ADD COLUMN capabilities_json TEXT;
