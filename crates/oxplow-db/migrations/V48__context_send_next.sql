-- One basket per send. A thread's next prompt carries one basket: its own
-- ("This Thread"), or the one named basket set to go next (`next_thread_id`;
-- at most one per thread, and a basket is next for one thread at a time).
-- After a send the choice is cleared, back to This Thread. Attachments (several
-- baskets riding along with the thread's own) are gone; the choices are
-- cleared, since a choice lasts one send anyway.
DROP INDEX context_basket_attached;
ALTER TABLE context_basket RENAME COLUMN attached_thread_id TO next_thread_id;
UPDATE context_basket SET next_thread_id = NULL;
CREATE UNIQUE INDEX context_basket_next ON context_basket (next_thread_id) WHERE next_thread_id IS NOT NULL;
