-- A context-basket item's note: what the person wants done about the page
-- or the text it holds, typed when they added it or later while it waits.
-- It rides the item into the agent's prompt (`agent_context::basket_block`).
ALTER TABLE thread_context_item ADD COLUMN note TEXT;
