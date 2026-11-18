-- A text item from a place with no lines to read back (a terminal's
-- scrollback) keeps the lines around it: the agent sees what the selection
-- was part of.
ALTER TABLE thread_context_item ADD COLUMN around TEXT;
