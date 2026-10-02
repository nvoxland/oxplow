-- P7.B5: co-change surprises left core. oxplow-analytics computes them as
-- its models `co_change_pair` (materialized over the commit index) and
-- `change_co_change` (a change's surprising files), so core neither
-- builds the history nor stores the rows.
DROP TABLE change_co_change;
