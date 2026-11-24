-- What each thread's next prompt carries: the items of the named basket set
-- next for it while that holds something, else its own that are not held
-- (put back). The one statement of the rule: the prompt reads it, and so do
-- the Context panel, its badge and the Context page.
SELECT t.id AS thread_id, c.id AS item_id, b.name AS basket
FROM ref('thread') t
JOIN source('context_item') c ON CASE
  WHEN EXISTS (SELECT 1 FROM source('context_item') n
                 JOIN source('context_basket') nb ON nb.id = n.basket_id
                WHERE nb.next_thread_id = t.id)
  THEN c.basket_id IN (SELECT nb.id FROM source('context_basket') nb WHERE nb.next_thread_id = t.id)
  ELSE c.thread_id = t.id AND c.held_at IS NULL
END
LEFT JOIN source('context_basket') b ON b.id = c.basket_id
