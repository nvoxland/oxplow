-- P7.B5: the "look here first" score left core. oxplow-analytics computes
-- it as its model `change_interest` over `change_file` and
-- `change_function` (the same formula), so core stores only what needs
-- two revisions' trees. `v_change_file` is version 2 without them.
ALTER TABLE change_file DROP COLUMN interest;
ALTER TABLE change_file DROP COLUMN interest_reasons;
