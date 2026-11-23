-- An item Put Back returned to a thread's own basket is held: the thread's
-- next prompt doesn't carry it until the person presses Send Next on This
-- Thread, which clears `held_at`. NULL: not held.
ALTER TABLE context_item ADD COLUMN held_at TEXT;
