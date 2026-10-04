-- tsk887: a failed attempt scheduled to be sent again by itself keeps
-- what it composed (its calls and events, JSON), and the automatic
-- attempt sends exactly that: composing afresh could change a step's
-- input, so its idempotency key, and a write that landed would be made
-- again.
ALTER TABLE effect_run ADD COLUMN resend_json TEXT;
