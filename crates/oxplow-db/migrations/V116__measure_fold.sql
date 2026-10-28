-- How a measure's series folds over time (`sum` | `last` | `first` | `avg` |
-- `none`): declared by the measure, applied by the one fold behind a chart's
-- bucket and a grid's headline. NULL is undeclared: a metric over it is a
-- check error. `temporal_semantics` stays: it scopes the current captures.
ALTER TABLE measure ADD COLUMN fold TEXT;

-- The rows already here keep what they read as: a level takes the latest
-- capture, everything else totals (a ratio or a mean re-divides its parts).
UPDATE measure SET fold = CASE temporal_semantics
    WHEN 'semi-additive' THEN 'last'
    ELSE 'sum'
END;
